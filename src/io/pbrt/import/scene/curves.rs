use crate::model::base::ParamSet;

#[derive(Default)]
pub struct CurvesState {
    pub curve_edition: String,
    pub curves: Vec<ParamSet>,
}

impl CurvesState {
    pub fn create_curve_edition(params: &ParamSet) -> String {

        //let curve_type = params.get()

        todo!()
    }
}
