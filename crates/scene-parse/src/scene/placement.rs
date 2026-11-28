use super::*;

fn transform_at(start: Mat4, end: Mat4, times: [f64; 2], time: f64) -> Option<Mat4> {
    if start == end {
        Some(start)
    } else if !times[0].is_finite() || !times[1].is_finite() || times[0] >= times[1] {
        None
    } else if time <= times[0] {
        Some(start)
    } else if time >= times[1] {
        Some(end)
    } else {
        crate::transform::interpolate(start, end, (time - times[0]) / (times[1] - times[0]))
    }
}

pub(super) fn placement_transforms(
    shape: &Shape,
    instance: Option<&InstanceUse>,
    supported_times: &[f64],
) -> Option<Vec<Mat4>> {
    let animated = shape.start_transform != shape.end_transform
        || instance.is_some_and(|use_site| use_site.start_transform != use_site.end_transform);
    if !animated {
        return Some(vec![instance.map_or(shape.start_transform, |use_site| {
            use_site.start_transform * shape.start_transform
        })]);
    }
    supported_times
        .iter()
        .map(|time| {
            let shape_transform = transform_at(
                shape.start_transform,
                shape.end_transform,
                shape.times,
                *time,
            )?;
            let instance_transform = instance
                .map(|use_site| {
                    transform_at(
                        use_site.start_transform,
                        use_site.end_transform,
                        use_site.times,
                        *time,
                    )
                })
                .unwrap_or(Some(Mat4::identity()))?;
            Some(instance_transform * shape_transform)
        })
        .collect()
}
pub(super) fn transformed_bounds(bounds: &Bounds, transform: Mat4) -> Bounds {
    let mut world = Bounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    for corner in 0..8 {
        let point = std::array::from_fn(|axis| {
            if corner & (1 << axis) == 0 {
                bounds.min[axis]
            } else {
                bounds.max[axis]
            }
        });
        let point = transform.transform_point(point);
        for axis in 0..3 {
            world.min[axis] = world.min[axis].min(point[axis]);
            world.max[axis] = world.max[axis].max(point[axis]);
        }
    }
    world
}
