use termcolor::ColorSpec;

pub struct UserColorSpec { v: u8 }
pub struct Merged { path: ColorSpec }

impl UserColorSpec {
    pub fn to_color_spec(&self) -> ColorSpec {
        let mut spec = ColorSpec::default();
        self.merge_into(&mut spec);
        spec
    }
    fn merge_into(&self, cspec: &mut ColorSpec) {}
}

pub fn new(specs: &[UserColorSpec]) -> Merged {
    let mut merged = Merged { path: ColorSpec::default() };
    for spec in specs {
        spec.merge_into(&mut merged.path);
    }
    merged
}
