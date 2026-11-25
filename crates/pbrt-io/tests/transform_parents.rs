//! Transform-parent resolution: `Identity` / `Transform [m]`
//! resets are rewritten into the hierarchy without changing the evaluated
//! meaning. Every test parses the same PBRT text twice — unresolved, and
//! through `resolve_transform_parents` — then checks equivalence, the
//! "nothing remains" rule, and the shape of the rewrite.
mod support;

use project::{
    ActiveSamples, EntityKind, NodeId, NodeKind, ScopeKind, StateOperation,
    resolve_transform_parents,
};
use support::transform_parents::*;

// -- fixtures -----------------------------------------------------------------

#[test]
fn depth1_transform_in_a_group_is_replaced_in_place() {
    let c = case(&format!(
        "AttributeBegin\n{M_PBRT}\nShape \"sphere\" \"float radius\" [1]\nAttributeEnd"
    ));
    let groups = world_children(&c.after, c.scene);
    assert_eq!(groups.len(), 1, "no group is created in place");
    let kids = children(&c.after, c.scene, groups[0]);
    assert_eq!(kids.len(), 2, "no node is created in place");
    assert_in_place_concat(&c, kids[0]);
    assert_resolved_exact(&c);
}

#[test]
fn transform_within_an_object_definition_is_replaced_in_place() {
    let c = case(&format!(
        "AttributeBegin\nObjectBegin \"obj\"\n{M_PBRT}\n\
         Shape \"sphere\" \"float radius\" [0.5]\n\
         ObjectEnd\nObjectInstance \"obj\"\nAttributeEnd"
    ));
    let [root] = definition_roots(&c.after)[..] else {
        panic!("expected exactly one definition")
    };
    let body = children(&c.after, c.scene, root);
    assert_eq!(body.len(), 1, "the definition body is wrapped");
    assert_eq!(label(&c.after, c.scene, body[0]), "World Transform");
    let wrapped = children(&c.after, c.scene, body[0]);
    assert_eq!(
        labels_of(&c.after, c.scene, &wrapped),
        vec!["ConcatTransform", "Shape"],
        "an identity declaration leaves nothing to rebase"
    );
    assert_in_place_concat(&c, wrapped[0]);
    assert_resolved_exact(&c);
}

#[test]
fn in_place_reset_under_active_transform_start_time() {
    // Only the start sample is reset; the end sample keeps its translation,
    // so the identity check looks at the affected sample only.
    let c = case(&format!(
        "AttributeBegin\nActiveTransform EndTime\nTranslate 0 5 0\n\
         ActiveTransform StartTime\n{M_PBRT}\nActiveTransform All\n\
         Shape \"sphere\" \"float radius\" [1]\nAttributeEnd"
    ));
    let groups = world_children(&c.after, c.scene);
    assert_eq!(groups.len(), 1, "no group is created in place");
    let kids = children(&c.after, c.scene, groups[0]);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec![
            "ActiveTransform",
            "Translate",
            "ActiveTransform",
            "ConcatTransform",
            "ActiveTransform",
            "Shape"
        ]
    );
    assert_in_place_concat(&c, kids[3]);
    assert_resolved_exact(&c);
}

#[test]
fn whole_scope_moves_to_the_world_and_loses_its_head_transform() {
    let c = case(&format!(
        "AttributeBegin\nTranslate 9 0 0\nAttributeBegin\nTranslate 1 2 3\n{M_PBRT}\n\
         Shape \"sphere\" \"float radius\" [1]\nAttributeEnd\nAttributeEnd"
    ));
    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world),
        vec!["Group", "Group"],
        "the inner group moves right after its outer group"
    );
    let (outer, moved) = (world[0], world[1]);
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, moved)),
        vec!["ConcatTransform", "Shape"],
        "the head transform is deleted, not kept"
    );
    assert_in_place_concat(&c, children(&c.after, c.scene, moved)[0]);
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, outer)),
        vec!["Translate"],
        "the outer group keeps only its own head"
    );
    let moved_before = world_children(&c.before, c.scene)[0];
    let moved_kids_before = children(&c.before, c.scene, moved_before);
    let moved_scope = node_at(&c.before, c.scene, &moved_kids_before, "Group");
    let translate_node = node_at(
        &c.before,
        c.scene,
        &children(&c.before, c.scene, moved_scope),
        "Translate",
    );
    // The deleted head transform is gone from the whole document.
    assert!(
        !trees(&c.after, c.scene).contains(&translate_node),
        "the head transform node is removed"
    );
    assert_resolved_exact(&c);
}

#[test]
fn split_move_parks_the_tail_in_a_new_world_group() {
    let c = case(&format!(
        "AttributeBegin\n{SPLIT_BODY}Identity\nShape \"sphere\" \"float radius\" [1]\nAttributeEnd"
    ));
    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world),
        vec!["Group", "Group"],
        "the tail group is created right after the original group"
    );
    let (s, tail) = (world[0], world[1]);
    // The Identity reset is deleted in place of being moved.
    assert_eq!(
        children(&c.after, c.scene, s),
        vec![
            node_at(
                &c.before,
                c.scene,
                &children(&c.before, c.scene, s),
                "Translate"
            ),
            shape_by_radius(&c.after, c.scene, 0.25),
        ],
        "the head stays with the original group"
    );
    let tail_kids = children(&c.after, c.scene, tail);
    assert_eq!(
        labels_of(&c.after, c.scene, &tail_kids),
        vec!["Shape"],
        "a deleted Identity leaves just the tail object"
    );
    let shape = shape_by_radius(&c.after, c.scene, 1.0);
    assert_eq!(tail_kids, vec![shape]);
    match kind(&c.after, c.scene, tail) {
        NodeKind::Scope { owned_object, .. } => {
            assert_eq!(*owned_object, Some(shape), "the single-object tail owns it")
        }
        other => panic!("expected a scope, got {other:?}"),
    }
    assert_resolved_exact(&c);
}

#[test]
fn split_move_transfers_a_preset_owned_object_and_head_coordinate_system() {
    let mut c = case(&format!(
        "AttributeBegin\nTranslate 1 0 0\nCoordinateSystem \"c\"\n{M_PBRT}\n\
         Shape \"sphere\" \"float radius\" [1]\nAttributeEnd\n\
         CoordSysTransform \"c\"\nShape \"sphere\" \"float radius\" [2]"
    ));
    // Editor-made structure: the (sole visible) tail shape is the group's
    // owned object. Imports never set owned_object, so stage it here.
    let s = world_children(&c.before, c.scene)[0];
    let owned = shape_by_radius(&c.before, c.scene, 1.0);
    let document = std::sync::Arc::make_mut(c.before.scenes.get_mut(&c.scene).unwrap());
    match &mut document.nodes.get_mut(&s).unwrap().data {
        NodeKind::Scope { owned_object, .. } => *owned_object = Some(owned),
        other => panic!("expected a scope, got {other:?}"),
    }
    let mut after = c.before.clone();
    let diagnostics = resolve_transform_parents(&mut after, c.scene).unwrap();
    c.after = after;
    c.diagnostics = diagnostics;

    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world[..2]),
        vec!["Group", "Group"],
        "the used head coordinate system forces a split"
    );
    let (s, tail) = (world[0], world[1]);
    let moved = shape_by_radius(&c.after, c.scene, 1.0);
    assert_eq!(owned, moved, "the shape keeps its identity");
    match kind(&c.after, c.scene, tail) {
        NodeKind::Scope { owned_object, .. } => {
            assert_eq!(*owned_object, Some(moved), "the owned object moved")
        }
        other => panic!("expected a scope, got {other:?}"),
    }
    match kind(&c.after, c.scene, s) {
        NodeKind::Scope { owned_object, .. } => assert_eq!(*owned_object, None),
        other => panic!("expected a scope, got {other:?}"),
    }
    assert_resolved_exact(&c);
}

// -- materialization ----------------------------------------------------------

/// The shared materialization shape: graphics state set inside the outer
/// group reaches the tail only through inheritance, so the new tail group
/// must write it explicitly at its head. `outer` is the outer group's
/// label after import — "gold" when the state text names a material
/// directly (the group takes its name from that material).
fn materialization_case(
    outer: &str,
    state_text: &str,
    head_labels: &[&str],
) -> (Case, NodeId, Vec<NodeId>) {
    let c = case(&format!(
        "AttributeBegin\n{state_text}\nAttributeBegin\n{SPLIT_BODY}{M_PBRT}\n\
         Shape \"sphere\" \"float radius\" [1]\nAttributeEnd\nAttributeEnd"
    ));
    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world),
        vec![outer, "Group"],
        "world children labels"
    );
    let tail = world[1];
    let kids = children(&c.after, c.scene, tail);
    let labels = labels_of(&c.after, c.scene, &kids);
    assert_eq!(
        labels,
        head_labels
            .iter()
            .copied()
            .chain(["ConcatTransform", "Shape"])
            .collect::<Vec<_>>(),
        "the tail group's head"
    );
    assert_in_place_concat(&c, kids[head_labels.len()]);
    (c, tail, kids)
}

#[test]
fn split_move_materializes_an_inline_material() {
    let (c, _tail, kids) = materialization_case(
        "Group",
        "Material \"diffuse\" \"rgb reflectance\" [0.2 0.5 0.8]",
        &["NamedMaterial"],
    );
    match state_node_at(&c.after, c.scene, kids[0]) {
        StateOperation::Material(project::MaterialBinding::Resource(id)) => {
            assert_eq!(*id, resource_id(&c.before, "diffuse"));
        }
        other => panic!("expected a material write, got {other:?}"),
    }
    assert_resolved_exact(&c);
}

#[test]
fn split_move_materializes_a_named_material() {
    let (c, _tail, kids) = materialization_case(
        // The outer group names itself after the first NamedMaterial set
        // directly inside it; the tail group keeps "Group".
        "gold",
        "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] \
          \"spectrum eta\" [\"metal-Au-eta\"] \"spectrum k\" [\"metal-Au-k\"] \
          \"float roughness\" [0.1]\nNamedMaterial \"gold\"",
        &["NamedMaterial"],
    );
    match state_node_at(&c.after, c.scene, kids[0]) {
        StateOperation::Material(project::MaterialBinding::Resource(id)) => {
            assert_eq!(*id, resource_id(&c.before, "gold"));
        }
        other => panic!("expected a material write, got {other:?}"),
    }
    assert_resolved_exact(&c);
}

#[test]
fn split_move_materializes_a_medium_interface() {
    let (c, _tail, kids) = materialization_case(
        "Group",
        "MakeNamedMedium \"fog\" \"string type\" [\"homogeneous\"] \
          \"rgb sigma_a\" [0.1 0.1 0.1] \"rgb sigma_s\" [0.5 0.5 0.5] \
          \"float scale\" [0.5]\nMediumInterface \"fog\" \"\"",
        &["MediumInterface"],
    );
    match state_node_at(&c.after, c.scene, kids[0]) {
        StateOperation::Medium { inside, outside } => {
            assert_eq!(*inside, Some(resource_id(&c.before, "fog")));
            assert_eq!(*outside, None);
        }
        other => panic!("expected a medium write, got {other:?}"),
    }
    assert_resolved_exact(&c);
}

#[test]
fn split_move_materializes_the_color_space() {
    let (c, _tail, kids) =
        materialization_case("Group", "ColorSpace \"aces2065-1\"", &["ColorSpace"]);
    match state_node_at(&c.after, c.scene, kids[0]) {
        StateOperation::ColorSpace(name) => assert_eq!(name, "aces2065-1"),
        other => panic!("expected a color space write, got {other:?}"),
    }
    assert_resolved_exact(&c);
}

#[test]
fn split_move_materializes_reverse_orientation_parity() {
    // An odd number of flips survives the reset, so the tail writes one.
    let (c, _tail, kids) =
        materialization_case("Group", "ReverseOrientation", &["ReverseOrientation"]);
    assert!(matches!(
        state_node_at(&c.after, c.scene, kids[0]),
        StateOperation::ReverseOrientation
    ));
    assert_resolved_exact(&c);

    // Even parity: the flips cancel out, nothing is written.
    let (c, _tail, kids) =
        materialization_case("Group", "ReverseOrientation\nReverseOrientation", &[]);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["ConcatTransform", "Shape"],
        "even parity needs no reverse-orientation write"
    );
    assert_resolved_exact(&c);
}

#[test]
fn split_move_duplicates_the_active_area_light() {
    let (c, _tail, kids) = materialization_case(
        "Group",
        "AreaLightSource \"diffuse\" \"blackbody L\" [6500] \"float scale\" [4]",
        &["AreaLightSource"],
    );
    // The write is a duplicate of the outer group's area light: same
    // implementation and parameters, but a new node.
    let written = match kind(&c.after, c.scene, kids[0]) {
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation,
            parameters,
            ..
        } => (implementation.clone(), parameters.clone()),
        other => panic!("expected a duplicated area light, got {other:?}"),
    };
    let outer = world_children(&c.after, c.scene)[0];
    let original = children(&c.after, c.scene, outer)
        .into_iter()
        .find(|id| {
            matches!(
                kind(&c.after, c.scene, *id),
                NodeKind::Entity {
                    entity: EntityKind::AreaLight,
                    ..
                }
            )
        })
        .expect("the original area light stays with the outer group");
    assert_ne!(kids[0], original, "the duplicate is a new node");
    match kind(&c.after, c.scene, original) {
        NodeKind::Entity {
            implementation,
            parameters,
            ..
        } => assert_eq!((implementation.clone(), parameters.clone()), written),
        other => panic!("expected an area light, got {other:?}"),
    }
    assert_resolved_exact(&c);
}

#[test]
fn split_move_writes_the_active_transform_selector() {
    let (c, _tail, kids) = materialization_case(
        "Group",
        "ActiveTransform StartTime\nTranslate 5 0 0\nActiveTransform EndTime\nRotate 30 0 1 0",
        &["ActiveTransform", "ConcatTransform", "ActiveTransform"],
    );
    // Selector states are preserved verbatim where they still apply.
    match state_node_at(&c.after, c.scene, kids[0]) {
        StateOperation::ActiveTransform(ActiveSamples::Start) => {}
        other => panic!("expected a start selector, got {other:?}"),
    }
    match state_node_at(&c.after, c.scene, kids[2]) {
        StateOperation::ActiveTransform(ActiveSamples::End) => {}
        other => panic!("expected an end selector, got {other:?}"),
    }
    assert_eq!(
        concat_matrix(&c.after, c.scene, kids[1]),
        translate(5.0, 0.0, 0.0)
    );
    assert_resolved_exact(&c);
}

#[test]
fn partial_sample_move_bakes_the_unaffected_sample() {
    let c = case(&format!(
        "AttributeBegin\nShape \"sphere\" \"float radius\" [0.25]\n\
         ActiveTransform StartTime\nTranslate 9 0 0\nActiveTransform EndTime\n\
         Rotate 30 0 1 0\nActiveTransform StartTime\n{M_PBRT}\nActiveTransform All\n\
         Shape \"sphere\" \"float radius\" [1]\nAttributeEnd"
    ));
    let world = world_children(&c.after, c.scene);
    assert_eq!(labels_of(&c.after, c.scene, &world), vec!["Group", "Group"]);
    let tail = world[1];
    let kids = children(&c.after, c.scene, tail);
    // The reset applies to the start sample only, so the start sample is
    // rewritten by the reset itself; the unaffected end sample is carried
    // over with an explicit write, and the selector ends at the reset's own
    // value.
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec![
            "ActiveTransform",
            "ConcatTransform",
            "ActiveTransform",
            "ConcatTransform",
            "ActiveTransform",
            "Shape"
        ]
    );
    assert_eq!(
        concat_matrix(&c.after, c.scene, kids[1]),
        rotation(30.0),
        "the unaffected sample's transform before the reset is baked"
    );
    assert_in_place_concat(&c, kids[3]);
    assert_eq!(kids[5], shape_by_radius(&c.after, c.scene, 1.0));
    assert_resolved_exact(&c);
}

#[test]
fn nested_resets_resolve_in_document_order() {
    let inner = format!(
        "AttributeBegin\nTranslate 2 0 0\n{M_PBRT}\n\
         Shape \"sphere\" \"float radius\" [1]\nAttributeEnd\n"
    );
    let c = case(&format!(
        "AttributeBegin\nShape \"sphere\" \"float radius\" [0.25]\nTranslate 1 0 0\n{M_PBRT}\n{inner}AttributeEnd"
    ));
    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world),
        vec!["Group", "Group", "Group"],
        "outer split, then the inner whole move after it"
    );
    let (outer, first, inner) = (world[0], world[1], world[2]);
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, outer)),
        vec!["Shape", "Translate"],
        "the split group keeps its head"
    );
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, first)),
        vec!["ConcatTransform"]
    );
    let inner_kids = children(&c.after, c.scene, inner);
    assert_eq!(
        labels_of(&c.after, c.scene, &inner_kids),
        vec!["ConcatTransform", "Shape"],
        "the inner head transform is deleted by the whole move"
    );
    assert_in_place_concat(&c, inner_kids[0]);
    assert_resolved_exact(&c);
}

#[test]
fn definition_reset_bakes_the_inverse_of_a_non_identity_declaration() {
    let c = case(&format!(
        "AttributeBegin\nTranslate 1 2 3\nObjectBegin \"obj\"\n{M_PBRT}\n\
         Shape \"sphere\" \"float radius\" [0.5]\nObjectEnd\nAttributeEnd\n\
         ObjectInstance \"obj\""
    ));
    let [root] = definition_roots(&c.after)[..] else {
        panic!("expected exactly one definition")
    };
    let wrapper = children(&c.after, c.scene, root)[0];
    assert_eq!(label(&c.after, c.scene, wrapper), "World Transform");
    let kids = children(&c.after, c.scene, wrapper);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["ConcatTransform", "ConcatTransform", "Shape"]
    );
    assert_eq!(
        concat_matrix(&c.after, c.scene, kids[0]),
        project::math::inverse(project::math::Mat4(translate(1.0, 2.0, 3.0)))
            .unwrap()
            .0,
        "the declaration transform is inverted into the head"
    );
    assert_in_place_concat(&c, kids[1]);
    assert_resolved_baked(&c);
}

#[test]
fn whole_move_bridge_starts_from_the_head_active_selector() {
    // The second reset's whole move keeps the head's `ActiveTransform
    // StartTime`; its bridge must not assume `All` there, or the end sample
    // of the shared inverse(D) write is lost.
    let c = case(
        "AttributeBegin\nTranslate 1 2 3\nObjectBegin \"obj\"\n\
         AttributeBegin\nActiveTransform EndTime\nIdentity\n\
         ActiveTransform StartTime\nIdentity\n\
         Shape \"sphere\" \"float radius\" [0.5]\nAttributeEnd\n\
         ObjectEnd\nAttributeEnd\nObjectInstance \"obj\"",
    );
    assert_resolved_baked(&c);
}

#[test]
fn definition_reset_bakes_an_animated_declaration_per_sample() {
    let text = format!(
        "Camera \"perspective\" \"float fov\" [40]\n\
         Film \"rgb\" \"integer xresolution\" [320] \"integer yresolution\" [240]\n\
         TransformTimes 0 1\nWorldBegin\n\
         AttributeBegin\nActiveTransform StartTime\nTranslate 1 0 0\n\
         ActiveTransform EndTime\nTranslate 0 0 5\nActiveTransform All\n\
         ObjectBegin \"obj\"\n{M_PBRT}\n\
         Shape \"sphere\" \"float radius\" [0.5]\nObjectEnd\nAttributeEnd\n\
         ObjectInstance \"obj\"\nWorldEnd"
    );
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    std::fs::write(&source, &text).unwrap();
    let c = resolve_case(&source);
    let [root] = definition_roots(&c.after)[..] else {
        panic!("expected exactly one definition")
    };
    let wrapper = children(&c.after, c.scene, root)[0];
    let kids = children(&c.after, c.scene, wrapper);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec![
            "ActiveTransform",
            "ConcatTransform",
            "ActiveTransform",
            "ConcatTransform",
            "ActiveTransform",
            "ConcatTransform",
            "Shape"
        ]
    );
    assert_eq!(
        concat_matrix(&c.after, c.scene, kids[1]),
        project::math::inverse(project::math::Mat4(translate(1.0, 0.0, 0.0)))
            .unwrap()
            .0
    );
    assert_eq!(
        concat_matrix(&c.after, c.scene, kids[3]),
        project::math::inverse(project::math::Mat4(translate(0.0, 0.0, 5.0)))
            .unwrap()
            .0
    );
    assert_in_place_concat(&c, kids[5]);
    assert_resolved_baked(&c);
}

#[test]
fn world_level_transforms_are_wrapped_in_one_group() {
    let c = case(
        "LightSource \"infinite\" \"rgb L\" [0.1 0.1 0.1]\nTranslate 1 2 3\n\
         AttributeBegin\nShape \"sphere\" \"float radius\" [1]\nAttributeEnd\n\
         Rotate 30 0 1 0\nShape \"disk\" \"float radius\" [0.5]",
    );
    let world = world_children(&c.after, c.scene);
    assert_eq!(world.len(), 2, "the light stays, the rest is wrapped");
    assert_eq!(label(&c.after, c.scene, world[0]), "LightSource");
    assert_eq!(label(&c.after, c.scene, world[1]), "World Transform");
    let kids = children(&c.after, c.scene, world[1]);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["Translate", "Group", "Rotate", "Shape"],
        "the wrapped children keep their order"
    );
    assert_eq!(kids[0], world_children(&c.before, c.scene)[1]);
    assert_eq!(kids[1], world_children(&c.before, c.scene)[2]);
    assert_resolved_exact(&c);
}

#[test]
fn transform_begin_leaks_materials_and_media_to_later_siblings() {
    let c = case(
        r#"
AttributeBegin
MakeNamedMedium "fog" "string type" ["homogeneous"]
  "rgb sigma_a" [0.1 0.1 0.1] "rgb sigma_s" [0.5 0.5 0.5] "float scale" [0.5]
MakeNamedMaterial "gold" "string type" ["conductor"]
  "spectrum eta" ["metal-Au-eta"] "spectrum k" ["metal-Au-k"] "float roughness" [0.1]
TransformBegin
NamedMaterial "gold"
MediumInterface "fog" ""
Shape "sphere" "float radius" [0.5]
TransformEnd
Shape "sphere" "float radius" [1]
AttributeEnd
"#,
    );
    let group = world_children(&c.after, c.scene)[0];
    let kids = children(&c.after, c.scene, group);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["gold", "NamedMaterial", "MediumInterface", "Shape"],
        "the material and medium are written right after the converted \
         scope, which names itself after its first NamedMaterial (C4)"
    );
    match kind(&c.after, c.scene, kids[0]) {
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            ..
        } => {}
        other => panic!("the transform scope became {other:?}"),
    }
    match state_node_at(&c.after, c.scene, kids[1]) {
        StateOperation::Material(project::MaterialBinding::Resource(id)) => {
            assert_eq!(*id, resource_id(&c.before, "gold"));
        }
        other => panic!("expected a material write, got {other:?}"),
    }
    match state_node_at(&c.after, c.scene, kids[2]) {
        StateOperation::Medium { inside, outside } => {
            assert_eq!(*inside, Some(resource_id(&c.before, "fog")));
            assert_eq!(*outside, None);
        }
        other => panic!("expected a medium write, got {other:?}"),
    }
    assert_resolved_exact(&c);
}

#[test]
fn transform_begin_leaks_color_spaces_area_lights_and_parity() {
    let c = case(
        r#"
AttributeBegin
TransformBegin
AreaLightSource "diffuse" "blackbody L" [6500] "float scale" [4]
ColorSpace "aces2065-1"
ReverseOrientation
Shape "sphere" "float radius" [0.5]
TransformEnd
Shape "sphere" "float radius" [1]
TransformBegin
ReverseOrientation
ReverseOrientation
Shape "sphere" "float radius" [2]
TransformEnd
Shape "sphere" "float radius" [3]
AttributeEnd
"#,
    );
    let group = world_children(&c.after, c.scene)[0];
    let kids = children(&c.after, c.scene, group);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec![
            "Group",
            "ColorSpace",
            "ReverseOrientation",
            "AreaLightSource",
            "Shape",
            "Group",
            "Shape",
        ],
        "odd parity and new state are written, even parity writes nothing"
    );
    match state_node_at(&c.after, c.scene, kids[1]) {
        StateOperation::ColorSpace(name) => assert_eq!(name, "aces2065-1"),
        other => panic!("expected a color space write, got {other:?}"),
    }
    let original = children(&c.after, c.scene, kids[0])[0];
    let (write_id, write) = match kind(&c.after, c.scene, kids[3]) {
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation,
            parameters,
            ..
        } => (kids[3], (implementation.clone(), parameters.clone())),
        other => panic!("expected a duplicated area light, got {other:?}"),
    };
    match kind(&c.after, c.scene, original) {
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation,
            parameters,
            ..
        } => {
            assert_ne!(write_id, original, "the area light is duplicated");
            assert_eq!((implementation.clone(), parameters.clone()), write);
        }
        other => panic!("expected an area light, got {other:?}"),
    }
    assert_resolved_exact(&c);
}

// -- camera poses and coordinate systems -------------------------------------

fn pose_slot_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < BAKED_EPSILON,
        "{what}: {actual} vs {expected}"
    );
}

#[test]
fn a_camera_pose_replaces_the_options_transform_stream() {
    let c = camera_case(
        "LookAt 3 4 -6  0 0 0  0 1 0",
        "Shape \"sphere\" \"float radius\" [1]",
    );
    assert_eq!(
        options_labels(&c.after, c.scene),
        vec!["Camera", "Film"],
        "the whole Options transform stream disappears"
    );
    let [camera] = options_cameras(&c.after, c.scene)[..] else {
        panic!("expected one camera")
    };
    let pose = pose_of(&c.after, c.scene, camera);
    assert_eq!(pose.start, pose.end, "a static camera repeats one sample");
    pose_slot_close(pose.start[3], 3.0, "camera-to-world x");
    pose_slot_close(pose.start[7], 4.0, "camera-to-world y");
    pose_slot_close(pose.start[11], -6.0, "camera-to-world z");
    assert_resolved_exact(&c);
}

#[test]
fn a_mirrored_camera_pose_survives_a_negative_scale() {
    let c = camera_case(
        "Scale -1 1 1\nLookAt 1 0 -3  0 0 0  0 1 0",
        "Shape \"sphere\" \"float radius\" [1]",
    );
    assert_eq!(options_labels(&c.after, c.scene), vec!["Camera", "Film"]);
    let [camera] = options_cameras(&c.after, c.scene)[..] else {
        panic!("expected one camera")
    };
    let pose = pose_of(&c.after, c.scene, camera);
    pose_slot_close(pose.start[3], 1.0, "camera-to-world x");
    pose_slot_close(pose.start[11], -3.0, "camera-to-world z");
    assert_resolved_exact(&c);
}

#[test]
fn an_animated_camera_pose_records_both_samples() {
    let c = camera_case(
        "TransformTimes 1 2\nActiveTransform StartTime\nTranslate 1 0 0\n\
         ActiveTransform EndTime\nTranslate 0 0 4\nActiveTransform All",
        "Shape \"sphere\" \"float radius\" [1]",
    );
    assert_eq!(
        options_labels(&c.after, c.scene),
        vec!["TransformTimes", "Camera", "Film"],
        "the times setting is not a transform state"
    );
    let [camera] = options_cameras(&c.after, c.scene)[..] else {
        panic!("expected one camera")
    };
    let pose = pose_of(&c.after, c.scene, camera);
    assert_ne!(pose.start, pose.end, "the two samples differ");
    assert_eq!(pose.start[3], -1.0, "inverse of the start translate");
    assert_eq!(pose.end[11], -4.0, "inverse of the end translate");
    assert_resolved_exact(&c);
}

#[test]
fn every_options_camera_carries_its_own_pose() {
    let text = "LookAt 0 0 -5  0 0 0  0 1 0\n\
                Camera \"perspective\" \"float fov\" [40]\n\
                Translate 2 0 0\n\
                Camera \"orthographic\"\n\
                Film \"rgb\" \"integer xresolution\" [320] \"integer yresolution\" [240]\n\
                WorldBegin\nShape \"sphere\" \"float radius\" [1]\nWorldEnd\n";
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    std::fs::write(&source, text).unwrap();
    let c = resolve_case(&source);
    let cameras = options_cameras(&c.after, c.scene);
    let [first, second] = cameras[..] else {
        panic!("expected two cameras")
    };
    assert_eq!(
        scene_doc(&c.after, c.scene).render_camera,
        Some(second),
        "the last Options camera renders"
    );
    let (first_pose, second_pose) = (
        pose_of(&c.after, c.scene, first),
        pose_of(&c.after, c.scene, second),
    );
    assert_ne!(first_pose.start, second_pose.start);
    pose_slot_close(first_pose.start[11], -5.0, "the first camera looks from -5");
    pose_slot_close(
        second_pose.start[3],
        -2.0,
        "the right-concat translate shifts the second camera",
    );
    pose_slot_close(second_pose.start[11], -5.0, "second camera depth");
    assert_eq!(
        options_labels(&c.after, c.scene),
        vec!["Camera", "Camera", "Film"]
    );
    assert_resolved_exact(&c);
}

#[test]
fn an_options_medium_declaration_keeps_its_recorded_state() {
    let c = camera_case(
        "LookAt 0 0 -5  0 0 0  0 1 0\nMakeNamedMedium \"fog\" \"string type\" [\"homogeneous\"] \
         \"rgb sigma_a\" [0.1 0.1 0.1] \"rgb sigma_s\" [0.5 0.5 0.5]",
        "MediumInterface \"fog\" \"\"\nShape \"sphere\" \"float radius\" [1]",
    );
    let medium = resource_id(&c.before, "fog");
    assert_eq!(
        c.before.resources.get(&medium).unwrap().declared_in,
        c.after.resources.get(&medium).unwrap().declared_in,
        "the medium's declaration state is already recorded"
    );
    assert_eq!(options_labels(&c.after, c.scene), vec!["Camera", "Film"]);
    assert_resolved_exact(&c);
}

#[test]
fn a_camera_relative_group_attaches_to_the_render_camera() {
    let c = case("CoordSysTransform \"camera\"\nLightSource \"point\" \"rgb I\" [10 10 10]");
    let world = world_children(&c.after, c.scene);
    let [group] = world[..] else {
        panic!("expected one world child, got {world:?}")
    };
    assert_eq!(label(&c.after, c.scene, group), "Group");
    assert_eq!(
        parent_camera_of(&c.after, c.scene, group),
        scene_doc(&c.after, c.scene).render_camera,
        "the group attaches to the render camera"
    );
    let kids = children(&c.after, c.scene, group);
    assert_eq!(labels_of(&c.after, c.scene, &kids), vec!["LightSource"]);
    assert!(c.diagnostics.is_empty(), "the attachment is exact");
    assert_resolved_exact(&c);
}

#[test]
fn a_definition_cannot_host_a_camera_relative_group() {
    let c = camera_case(
        "LookAt 1 2 -3  0 0 0  0 1 0",
        "ObjectBegin \"obj\"\nCoordSysTransform \"camera\"\n\
         Shape \"sphere\" \"float radius\" [0.5]\nObjectEnd\nObjectInstance \"obj\"",
    );
    let [root] = definition_roots(&c.after)[..] else {
        panic!("expected exactly one definition")
    };
    let kids = children(&c.after, c.scene, root);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["ConcatTransform", "Shape"],
        "the camera pose is baked into the definition's head"
    );
    let [camera] = options_cameras(&c.after, c.scene)[..] else {
        panic!("expected one camera")
    };
    let pose = pose_of(&c.after, c.scene, camera);
    assert_eq!(
        concat_matrix(&c.after, c.scene, kids[0]),
        pose.start,
        "the baked matrix is the camera pose"
    );
    assert_eq!(
        c.diagnostics
            .iter()
            .filter(
                |d| d.code == "coordinate_system_fixed" && d.message.contains("object definition")
            )
            .count(),
        1,
        "one diagnostic fixes the coordinate system"
    );
    assert!(matches!(
        kind(&c.after, c.scene, kids[1]),
        NodeKind::Entity {
            entity: EntityKind::Shape,
            ..
        }
    ));
    assert_resolved_exact(&c);
}

#[test]
fn a_used_coordinate_system_becomes_a_group() {
    let c = case(
        "AttributeBegin\nTranslate 2 3 4\nCoordinateSystem \"stage\"\nAttributeEnd\n\
         AttributeBegin\nScale 2 2 2\nCoordSysTransform \"stage\"\n\
         Shape \"sphere\" \"float radius\" [1]\nShape \"sphere\" \"float radius\" [2]\nAttributeEnd",
    );
    let world = world_children(&c.after, c.scene);
    assert_eq!(labels_of(&c.after, c.scene, &world), vec!["Group"]);
    let definer = world[0];
    let kids = children(&c.after, c.scene, definer);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["Translate", "stage"],
        "the used definition becomes its group"
    );
    let attached = children(&c.after, c.scene, kids[1]);
    assert_eq!(labels_of(&c.after, c.scene, &attached), vec!["Group"]);
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, attached[0])),
        vec!["Shape", "Shape"],
        "the whole user scope moves; the reset canceled its head scale"
    );
    assert!(c.diagnostics.is_empty());
    assert_resolved_exact(&c);
}

#[test]
fn two_uses_of_one_coordinate_system_share_its_group() {
    let c = case(
        "AttributeBegin\nTranslate 2 3 4\nCoordinateSystem \"stage\"\nAttributeEnd\n\
         AttributeBegin\nCoordSysTransform \"stage\"\n\
         Shape \"sphere\" \"float radius\" [1]\nAttributeEnd\n\
         AttributeBegin\nCoordSysTransform \"stage\"\n\
         Shape \"sphere\" \"float radius\" [2]\nAttributeEnd",
    );
    let world = world_children(&c.after, c.scene);
    let kids = children(&c.after, c.scene, world[0]);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["Translate", "stage"]
    );
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, kids[1])),
        vec!["Group", "Group"],
        "both uses attach under the one group"
    );
    assert!(c.diagnostics.is_empty());
    assert_resolved_exact(&c);
}

#[test]
fn two_uses_of_one_options_coordinate_system_share_its_group() {
    let c = camera_case(
        "Translate 2 0 0\nCoordinateSystem \"camplane\"",
        "AttributeBegin\nCoordSysTransform \"camplane\"\nShape \"sphere\" \"float radius\" [1]\nAttributeEnd\n\
         AttributeBegin\nCoordSysTransform \"camplane\"\nShape \"sphere\" \"float radius\" [2]\nAttributeEnd",
    );
    let world = world_children(&c.after, c.scene);
    assert_eq!(labels_of(&c.after, c.scene, &world), vec!["camplane"]);
    assert!(c.diagnostics.is_empty());
    assert_resolved_exact(&c);
}

#[test]
fn a_redefined_coordinate_system_attaches_to_the_active_definition() {
    let c = case(
        "AttributeBegin\nTranslate 1 0 0\nCoordinateSystem \"d\"\nTranslate 5 0 0\n\
         CoordinateSystem \"d\"\nAttributeEnd\n\
         AttributeBegin\nCoordSysTransform \"d\"\n\
         Shape \"sphere\" \"float radius\" [0.5]\nAttributeEnd",
    );
    let world = world_children(&c.after, c.scene);
    assert_eq!(labels_of(&c.after, c.scene, &world), vec!["Group"]);
    let definer = world[0];
    let kids = children(&c.after, c.scene, definer);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["Translate", "Translate", "d"],
        "only the definition active at the use keeps a group"
    );
    let attached = children(&c.after, c.scene, kids[2]);
    assert_eq!(labels_of(&c.after, c.scene, &attached), vec!["Group"]);
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, attached[0])),
        vec!["Shape"]
    );
    assert!(c.diagnostics.is_empty());
    assert_resolved_exact(&c);
}

#[test]
fn an_options_coordinate_system_becomes_a_world_group() {
    let c = camera_case(
        "Translate 2 0 0\nCoordinateSystem \"camplane\"",
        "CoordSysTransform \"camplane\"\nShape \"sphere\" \"float radius\" [1]",
    );
    assert_eq!(options_labels(&c.after, c.scene), vec!["Camera", "Film"]);
    let world = world_children(&c.after, c.scene);
    assert_eq!(labels_of(&c.after, c.scene, &world), vec!["camplane"]);
    let kids = children(&c.after, c.scene, world[0]);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["ConcatTransform", "Group"],
        "the definition's Options-time transform is written into the head"
    );
    assert_eq!(
        concat_matrix(&c.after, c.scene, kids[0]),
        translate(2.0, 0.0, 0.0)
    );
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, kids[1])),
        vec!["Shape"]
    );
    assert!(c.diagnostics.is_empty());
    assert_resolved_exact(&c);
}

#[test]
fn an_unused_coordinate_system_definition_disappears() {
    let c = case(
        "AttributeBegin\nTranslate 1 0 0\nCoordinateSystem \"unused\"\nAttributeEnd\n\
         Shape \"sphere\" \"float radius\" [1]",
    );
    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world),
        vec!["Group", "Shape"],
        "the unused definition's group disappears entirely"
    );
    let kids = children(&c.after, c.scene, world[0]);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["Translate"],
        "an unused definition is simply deleted"
    );
    assert!(c.diagnostics.is_empty());
    assert_resolved_exact(&c);
}

#[test]
fn a_cross_tree_coordinate_use_bakes_the_matrix() {
    let c = case(
        "AttributeBegin\nTranslate 3 0 0\nCoordinateSystem \"w0\"\nAttributeEnd\n\
         ObjectBegin \"obj\"\nCoordSysTransform \"w0\"\n\
         Shape \"sphere\" \"float radius\" [0.4]\nObjectEnd\nObjectInstance \"obj\"",
    );
    let [root] = definition_roots(&c.after)[..] else {
        panic!("expected exactly one definition")
    };
    let kids = children(&c.after, c.scene, root);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["ConcatTransform", "Shape"]
    );
    assert_eq!(
        concat_matrix(&c.after, c.scene, kids[0]),
        translate(3.0, 0.0, 0.0),
        "the coordinate value is baked into the definition's head"
    );
    assert!(
        c.diagnostics
            .iter()
            .any(|d| d.code == "coordinate_system_fixed" && d.message.contains("w0"))
    );
    assert_resolved_exact(&c);
}

#[test]
fn a_group_inherited_material_bakes_the_matrix() {
    let c = case(
        "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"]\
         \"spectrum eta\" [\"metal-Au-eta\"] \"spectrum k\" [\"metal-Au-k\"]\n\
         AttributeBegin\nNamedMaterial \"gold\"\nTranslate 1 2 3\nCoordinateSystem \"matsys\"\nAttributeEnd\n\
         CoordSysTransform \"matsys\"\nShape \"sphere\" \"float radius\" [0.5]",
    );
    let world = world_children(&c.after, c.scene);
    let kids = children(&c.after, c.scene, world[1]);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["ConcatTransform", "Shape"],
        "the tail bakes the coordinate value instead of following the group"
    );
    assert_eq!(
        concat_matrix(&c.after, c.scene, kids[0]),
        translate(1.0, 2.0, 3.0)
    );
    assert!(
        c.diagnostics
            .iter()
            .any(|d| d.code == "coordinate_system_fixed" && d.message.contains("matsys"))
    );
    assert_resolved_exact(&c);
}

#[test]
fn a_group_inherited_area_light_bakes_the_matrix() {
    let c = case(
        "AttributeBegin\nAreaLightSource \"diffuse\" \"blackbody L\" [6500] \"float scale\" [2]\
         \nTranslate 1 2 3\nCoordinateSystem \"litsys\"\nShape \"sphere\" \"float radius\" [7]\
         \nAttributeEnd\nCoordSysTransform \"litsys\"\nShape \"sphere\" \"float radius\" [0.5]",
    );
    let world = world_children(&c.after, c.scene);
    let kids = children(&c.after, c.scene, world[1]);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["ConcatTransform", "Shape"],
        "a tail without the group's area light cannot follow it"
    );
    assert!(
        c.diagnostics
            .iter()
            .any(|d| d.code == "coordinate_system_fixed" && d.message.contains("litsys"))
    );
    assert_resolved_baked(&c);
}

#[test]
fn a_bare_world_coordinate_use_behaves_like_identity() {
    let c =
        case("Translate 4 0 0\nCoordSysTransform \"world\"\nShape \"sphere\" \"float radius\" [1]");
    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world),
        vec!["World Transform"]
    );
    let kids = children(&c.after, c.scene, world[0]);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["Shape"],
        "the use resets to the identity, canceling the wrapped translate"
    );
    assert!(c.diagnostics.is_empty());
    assert_resolved_exact(&c);
}

#[test]
fn a_later_reset_uses_the_landing_state_of_a_moved_scope() {
    let c = case(
        "AttributeBegin\nTranslate 5 0 0\nAttributeBegin\nTranslate 1 0 0\nIdentity\n\
         Shape \"sphere\" \"float radius\" [1]\nTranslate 7 0 0\nIdentity\n\
         Shape \"sphere\" \"float radius\" [2]\nAttributeEnd\nAttributeEnd",
    );
    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world),
        vec!["Group", "Group", "Group"],
        "the wrapper, the whole-moved scope, then the split tail"
    );
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, world[0])),
        vec!["Translate"]
    );
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, world[1])),
        vec!["Shape", "Translate"],
        "the split keeps the head states; the translate sits behind the only\
         \nshape, where it affects nothing (head transforms are deleted\
         \nonly when the scope itself moves)"
    );
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, world[2])),
        vec!["Shape"],
        "the split tail needs no bridge: its anchor starts at the identity"
    );
    assert_resolved_exact(&c);
}

#[test]
fn a_camera_relative_group_restores_the_ambient_for_later_siblings() {
    let c = camera_case(
        "LookAt 0 0 -5  0 0 0  0 1 0",
        "AttributeBegin\nCoordSysTransform \"camera\"\n\
         LightSource \"point\" \"rgb I\" [10 10 10]\nAttributeEnd\n\
         Shape \"sphere\" \"float radius\" [1]",
    );
    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world),
        vec!["Group", "Shape"],
        "the group stays at its position; the sibling keeps its place after"
    );
    assert_eq!(
        parent_camera_of(&c.after, c.scene, world[0]),
        scene_doc(&c.after, c.scene).render_camera
    );
    // The sibling sits at the world identity, not at the camera pose: the
    // group replaces the transform on entry, and restores on exit.
    assert_resolved_exact(&c);
}

#[test]
fn a_reset_inside_a_camera_relative_group_uses_the_ambient_after_it() {
    let c = camera_case(
        "LookAt 0 0 -5  0 0 0  0 1 0",
        "AttributeBegin\nShape \"sphere\" \"float radius\" [0.25]\nCoordSysTransform \"camera\"\n\
         LightSource \"point\" \"rgb I\" [10 10 10]\n\
         AttributeBegin\nTranslate 2 0 0\nIdentity\nShape \"sphere\" \"float radius\" [2]\
         \nAttributeEnd\nAttributeEnd",
    );
    let world = world_children(&c.after, c.scene);
    assert_eq!(
        labels_of(&c.after, c.scene, &world),
        vec!["Group", "Group", "Group"],
        "the split head, the camera-attached tail group, the inner whole move"
    );
    assert_eq!(
        parent_camera_of(&c.after, c.scene, world[1]),
        scene_doc(&c.after, c.scene).render_camera,
        "the blocking head forces the attachment into its own group"
    );
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, world[2])),
        vec!["Shape"],
        "the inner reset's bridge anchors at the group's ambient, not the pose"
    );
    assert_resolved_exact(&c);
}

#[test]
fn nested_transform_scopes_convert_even_when_the_innner_one_leaks_last() {
    // S1's rightmost child is S2; S2 leaks an area light as its stream ends,
    // and S1's leak reaches the shape after it. The conversion must compute
    // every scope's end state before re-kinding or inserting writes.
    let c = case(
        "AttributeBegin\nTransformBegin\nTransformBegin\n\
         AreaLightSource \"diffuse\" \"blackbody L\" [6500] \"float scale\" [2]\n\
         Shape \"sphere\" \"float radius\" [0.5]\n\
         TransformEnd\nTransformEnd\nShape \"sphere\" \"float radius\" [1]\nAttributeEnd",
    );
    let group = world_children(&c.after, c.scene)[0];
    // The inner scope's leak is written before the outer shape; the outer
    // scope's leak follows the nested scope, so output is innermost first.
    let kids = children(&c.after, c.scene, group);
    assert_eq!(
        labels_of(&c.after, c.scene, &kids),
        vec!["Group", "AreaLightSource", "Shape"],
        "S1's leak is written beside it"
    );
    assert_eq!(
        labels_of(&c.after, c.scene, &children(&c.after, c.scene, kids[0])),
        vec!["Group", "AreaLightSource"],
        "S2's leak is written inside S1"
    );
    assert_eq!(
        labels_of(
            &c.after,
            c.scene,
            &children(&c.after, c.scene, children(&c.after, c.scene, kids[0])[0])
        ),
        vec!["AreaLightSource", "Shape"],
        "S2 keeps its own body"
    );
    assert_resolved_exact(&c);
}

#[test]
fn an_unbound_camera_use_reports_an_error_instead_of_panicking() {
    let mut c = case("CoordSysTransform \"camera\"\nShape \"sphere\" \"float radius\" [1]");
    // A camera-less editor-built model never resolves: the snapshot
    // evaluation itself refuses the use, and the resolver reports it.
    let camera = *cameras(&c.before, c.scene).iter().next().unwrap();
    {
        let document = std::sync::Arc::make_mut(c.before.scenes.get_mut(&c.scene).unwrap());
        document.render_camera = None;
        document.options.retain(|id| *id != camera);
        assert!(document.nodes.remove(&camera).is_some());
    }
    let mut resolved = c.before.clone();
    let error = resolve_transform_parents(&mut resolved, c.scene).unwrap_err();
    assert!(
        error.to_string().contains("coordinate system"),
        "got: {error:?}"
    );
}
