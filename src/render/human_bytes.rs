use std::fmt::Display;

const SUFFIX: [&str; 9] = ["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
const UNIT: f64 = 1000.0;

pub struct HumanBytes(f64);

impl HumanBytes {
    pub fn new<T: Into<f64>>(bytes: T) -> Self {
        Self(bytes.into())
    }
}

impl Display for HumanBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let size = self.0;

        if size <= 0.0 {
            write!(f, "0 B")
        } else {
            let base = size.log10() / UNIT.log10();

            let v = (UNIT.powf(base - base.floor()) * 10.0).round() / 10.0;
            let s = SUFFIX[base.floor() as usize];
            write!(f, "{:.1} {}", v, s)
        }
    }
}
