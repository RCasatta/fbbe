use std::fmt::Display;

pub struct Plural<'a>(&'a str, usize);

impl<'a> Plural<'a> {
    pub fn new(name: &'a str, size: usize) -> Self {
        Plural(name, size)
    }
}
impl<'a> Display for Plural<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.1 == 1 {
            write!(f, "{}", self.0)
        } else {
            write!(f, "{}s", self.0)
        }
    }
}
