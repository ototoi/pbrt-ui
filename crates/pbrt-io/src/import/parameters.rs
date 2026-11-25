use super::*;
impl<F: Fn() -> bool> Builder<'_, '_, F> {
    pub(super) fn parameters(
        &mut self,
        d: &Directive,
        search_dirs: &[(usize, Vec<PathBuf>)],
        origin: &Origin,
        implementation: &str,
    ) -> Result<Vec<Parameter>> {
        if !crate::schema::supports(&d.name, implementation) {
            return Err(unsupported(crate::schema::unsupported_declaration_reason(
                &d.name,
                implementation,
            )));
        }
        let mut result = Vec::new();
        for (index, p) in d.parameters.iter().enumerate() {
            let dirs = search_dirs
                .iter()
                .rev()
                .find(|(start, _)| *start <= index)
                .map_or(&[][..], |(_, dirs)| dirs);
            let texts = || -> Result<Vec<String>> {
                p.values
                    .iter()
                    .map(|v| {
                        v.as_text()
                            .map(str::to_owned)
                            .ok_or_else(|| invalid(format!("expected string for {}", p.name)))
                    })
                    .collect()
            };
            let values = match p.value_type.as_str() {
                "integer" => ParameterValues::Integers(
                    p.value_tokens
                        .iter()
                        .map(|v| {
                            v.parse::<i64>().map_err(|_| {
                                invalid(format!("integer out of range or fractional: {v}"))
                            })
                        })
                        .collect::<Result<_>>()?,
                ),
                "bool" => ParameterValues::Booleans(
                    p.values
                        .iter()
                        .map(|v| match v.as_text() {
                            Some("true") => Ok(true),
                            Some("false") => Ok(false),
                            _ => Err(invalid("invalid boolean")),
                        })
                        .collect::<Result<_>>()?,
                ),
                "float" | "rgb" | "point2" | "point3" | "vector2" | "vector3" | "normal3"
                // r4's shorter spellings of vector3 / normal3; the parameter
                // table's defaults use them (FX4 review finding).
                | "vector" | "normal"
                | "blackbody" => self.floats(p)?,
                "texture" => ParameterValues::Resources {
                    kind: ResourceKind::Texture,
                    ids: texts()?
                        .iter()
                        .map(|name| self.lookup(ResourceKind::Texture, name))
                        .collect::<Result<_>>()?,
                },
                "spectrum" if p.values.iter().all(|v| v.as_float().is_some()) => self.floats(p)?,
                "spectrum" => {
                    let names = texts()?;
                    let builtin = names
                        .iter()
                        .all(|name| super::super::spectra::is_builtin(name));
                    if builtin {
                        ParameterValues::BuiltinSpectra(names)
                    } else {
                        if names
                            .iter()
                            .any(|name| super::super::spectra::is_builtin(name))
                        {
                            return Err(unsupported("mixed builtin and file spectra"));
                        }
                        // Every file becomes a shared SpectrumData
                        // resource. A file that cannot be parsed keeps the
                        // declaration importing on its legacy asset form and
                        // reports the file in a non-skipping diagnostic —
                        // the same fallback the open-time migration applies
                        // to unreadable spd assets.
                        let mut ids = Vec::with_capacity(names.len());
                        let mut broken = Vec::new();
                        for name in &names {
                            match self.spectrum_data(name, dirs, origin) {
                                Ok(id) => ids.push(id),
                                Err(Error::Invalid(reason)) | Err(Error::Unsupported(reason)) => {
                                    broken.push((name, reason));
                                }
                                Err(error) => return Err(error),
                            }
                        }
                        if broken.is_empty() {
                            ParameterValues::Resources {
                                kind: ResourceKind::SpectrumData,
                                ids,
                            }
                        } else {
                            for (name, reason) in broken {
                                self.warn(
                                    "spectrum",
                                    format!("spectrum file {name}: {reason}"),
                                    Some(origin.clone()),
                                );
                            }
                            ParameterValues::Assets(
                                names
                                    .iter()
                                    .map(|name| self.asset(name, dirs, origin))
                                    .collect::<Result<_>>()?,
                            )
                        }
                    }
                }
                "string"
                    if p.name == "materials"
                        && implementation == "mix"
                        && matches!(d.name.as_str(), "Material" | "MakeNamedMaterial") =>
                {
                    ParameterValues::Resources {
                        kind: ResourceKind::Material,
                        ids: texts()?
                            .iter()
                            .map(|name| self.lookup(ResourceKind::Material, name))
                            .collect::<Result<_>>()?,
                    }
                }
                "string" => {
                    let names = texts()?;
                    let input_file = crate::schema::input_file(&d.name, implementation, &p.name);
                    if input_file {
                        ParameterValues::Assets(
                            names
                                .iter()
                                .map(|name| self.asset(name, dirs, origin))
                                .collect::<Result<_>>()?,
                        )
                    } else {
                        let known = crate::schema::plain_string(&d.name, &p.name);
                        if !known {
                            return Err(unsupported(
                                crate::schema::unsupported_parameter_reason(
                                    &d.name,
                                    implementation,
                                    &p.name,
                                    "string",
                                ),
                            ));
                        }
                        ParameterValues::Strings(names)
                    }
                }
                _ => return Err(unsupported(format!("parameter type {}", p.value_type))),
            };
            let values = self.bulk(values, &p.name)?;
            result.push(Parameter {
                value_type: p.value_type.clone(),
                name: p.name.clone(),
                color_space: ColorSpaceBinding::Inherit,
                values,
            });
        }
        Ok(result)
    }
    fn floats(&self, p: &scene_parse::Parameter) -> Result<ParameterValues> {
        Ok(ParameterValues::Floats(
            p.values
                .iter()
                .map(|v| {
                    let n = v
                        .as_float()
                        .ok_or_else(|| invalid("nonnumeric float parameter"))?;
                    Ok(if n.is_nan() {
                        FloatValue::Nan
                    } else if n == f64::INFINITY {
                        FloatValue::PositiveInfinity
                    } else if n == f64::NEG_INFINITY {
                        FloatValue::NegativeInfinity
                    } else {
                        FloatValue::Finite(n)
                    })
                })
                .collect::<Result<_>>()?,
        ))
    }
    fn bulk(&mut self, values: ParameterValues, name: &str) -> Result<ParameterValues> {
        let (bytes, encoding, count) = match &values {
            ParameterValues::Floats(v) if v.len() > 4096 => (
                v.iter()
                    .flat_map(|n| {
                        let n = match n {
                            FloatValue::Finite(n) => *n,
                            FloatValue::Nan => f64::NAN,
                            FloatValue::PositiveInfinity => f64::INFINITY,
                            FloatValue::NegativeInfinity => f64::NEG_INFINITY,
                        };
                        n.to_le_bytes()
                    })
                    .collect::<Vec<_>>(),
                ArrayEncoding::F64Le,
                v.len(),
            ),
            ParameterValues::Integers(v) if v.len() > 4096 => (
                v.iter().flat_map(|n| n.to_le_bytes()).collect(),
                ArrayEncoding::I64Le,
                v.len(),
            ),
            ParameterValues::Booleans(v) if v.len() > 4096 => (
                v.iter().map(|n| u8::from(*n)).collect(),
                ArrayEncoding::BoolU8,
                v.len(),
            ),
            _ => return Ok(values),
        };
        let object = self.sources.store.put_object(bytes.as_slice())?;
        let asset =
            self.asset_for_object(object, media_type_for_array(), "Data", name, "array", None);
        Ok(ParameterValues::Array {
            asset,
            encoding,
            count: count as u64,
        })
    }
    fn asset(&mut self, name: &str, dirs: &[PathBuf], origin: &Origin) -> Result<AssetId> {
        let path = resolve(name, dirs)?;
        if let Some(id) = self.assets.get(&path) {
            return Ok(*id);
        }
        let object = self.sources.copy(&path)?;
        let media_type = media_type(&path);
        let folder = asset_folder(media_type);
        let stem = basename(&path);
        let extension = extension(&path);
        let id = self.asset_for_object(
            object,
            media_type,
            folder,
            &stem,
            extension,
            Some(AssetOrigin {
                logical_path: name.to_owned(),
                origin: Some(origin.clone()),
            }),
        );
        self.assets.insert(path, id);
        Ok(id)
    }
    /// Import one spd file as a SpectrumData resource (spectrum resource
    /// Parse its samples, share one resource per distinct file
    /// within one import, and never keep the file as an Asset. Malformed or
    /// oversized input is an Err the caller reports while skipping the
    /// declaration.
    fn spectrum_data(
        &mut self,
        name: &str,
        dirs: &[PathBuf],
        origin: &Origin,
    ) -> Result<ResourceId> {
        let path = resolve(name, dirs)?;
        if let Some(id) = self.spectra.get(&path) {
            return Ok(*id);
        }
        let bytes =
            read_spd(&path).map_err(|error| invalid(format!("spectrum file {name}: {error}")))?;
        let samples = parse_spd(&bytes)
            .map_err(|reason| invalid(format!("spectrum file {name}: {reason}")))?;
        let label = basename(&path);
        let id = ResourceId::new();
        let table = |values: Vec<f64>| {
            ParameterValues::Floats(
                values
                    .into_iter()
                    .map(FloatValue::Finite)
                    .collect::<Vec<FloatValue>>(),
            )
        };
        let wavelength_values = self.bulk(table(samples.wavelengths), "wavelengths")?;
        let sample_values = self.bulk(table(samples.values), "values")?;
        let resource = Resource {
            id,
            label: label.clone(),
            location: self.layout.take(
                project::layout::kind_folder(ResourceKind::SpectrumData),
                &label,
                project::layout::kind_extension(ResourceKind::SpectrumData),
            ),
            edition: Revision::initial(),
            kind: ResourceKind::SpectrumData,
            texture_value_type: None,
            implementation: "piecewise".into(),
            parameters: vec![
                Parameter {
                    value_type: "float".into(),
                    name: "wavelengths".into(),
                    color_space: ColorSpaceBinding::Inherit,
                    values: wavelength_values,
                },
                Parameter {
                    value_type: "float".into(),
                    name: "values".into(),
                    color_space: ColorSpaceBinding::Inherit,
                    values: sample_values,
                },
            ],
            definition_scope: None,
            declared_in: None,
            origin: Some(origin.clone()),
        };
        self.document.resources.insert(id, resource);
        self.spectra.insert(path, id);
        self.added_spectra.push(id);
        Ok(id)
    }
    fn asset_for_object(
        &mut self,
        object: ObjectRef,
        media_type: &str,
        folder: &str,
        stem: &str,
        extension: &str,
        origin: Option<AssetOrigin>,
    ) -> AssetId {
        if let Some(id) = self.object_assets.get(&object) {
            return *id;
        }
        let id = AssetId::new();
        let location = self.layout.take(folder, stem, extension);
        self.document.assets.insert(
            id,
            Asset {
                id,
                location,
                media_type: media_type.into(),
                sha256: object.sha256.clone(),
                size: object.size,
                origin,
            },
        );
        self.object_assets.insert(object, id);
        self.added_assets.push(id);
        id
    }
}
fn media_type_for_array() -> &'static str {
    "application/x-pbrt-ui-array"
}

/// One spd file read within the importer's size guard; the bound exists so
/// a giant "spectrum" file skips a declaration instead of being read whole.
fn read_spd(path: &std::path::Path) -> Result<Vec<u8>> {
    let size = std::fs::metadata(path)?.len();
    if size > project::MAX_SPD_FILE_BYTES {
        return Err(Error::Invalid(format!(
            "{} bytes exceeds the {} byte limit",
            size,
            project::MAX_SPD_FILE_BYTES
        )));
    }
    Ok(std::fs::read(path)?)
}
