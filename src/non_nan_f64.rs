use std::cmp::{Ordering, PartialOrd};

#[derive(PartialEq, PartialOrd)]
pub struct NonNan(f64);

#[allow(dead_code)]
impl NonNan {
    pub fn new(val: f64) -> Option<NonNan> {
        if val.is_nan() {
            None
        } else {
            Some(NonNan(val))
        }
    }

    pub fn get(&self) -> f64 {
        self.0
    }
}

impl Eq for NonNan {}

impl Ord for NonNan {
    fn cmp(&self, other: &NonNan) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}
