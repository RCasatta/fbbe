use super::Html;
use bitcoin::hex::DisplayHex;
use maud::{html, Render};

pub(crate) struct Witness<'a>(&'a bitcoin::Witness);

impl Render for Witness<'_> {
    fn render(&self) -> maud::Markup {
        // The following logic makes hex the witness elements, empty elements become "<empty>".
        // Moreover there is a deduplication logic where same consecutive elements like "00 00"
        // are shown as "00 2 times". This helps showing tx like
        // 73be398c4bdc43709db7398106609eea2a7841aaf3a4fa2000dc18184faa2a7e which contains
        // 500_001 empty push
        let mut witness = vec![];
        let mut count = 1;
        let w = self.0.to_vec();
        log::debug!("witness: {w:?}");

        let mut iter = w.into_iter();

        if let Some(mut before) = iter.next() {
            let mut last = None;
            for current in iter {
                if before != current {
                    push(before, &mut witness, count);
                    count = 1;
                } else {
                    count += 1;
                }

                last = Some(current.clone());
                before = current;
            }

            if witness.is_empty() {
                push(before, &mut witness, 1);
            } else if let Some(last) = last {
                push(last, &mut witness, count);
            }
        }

        html! {
            code {
                @for (i, el) in witness.iter().enumerate()  {
                    @if i != 0 {
                        " "
                    }
                    @if i % 2 == 0 {
                        span class="wit0" { (el) }
                    } @else {
                        span class="wit1" { (el) }
                    }
                }

            }
        }
    }
}

fn push(data: Vec<u8>, witness: &mut Vec<String>, count: i32) {
    if count == 1 {
        witness.push(hex_empty_long(&data));
    } else {
        witness.push(format!("{} {} times", hex_empty_long(&data), count));
    }
}

impl Html for bitcoin::Witness {
    fn html(&self) -> maud::Markup {
        Witness(self).render()
    }
}

impl<'a> From<&'a bitcoin::Witness> for Witness<'a> {
    fn from(a: &'a bitcoin::Witness) -> Self {
        Witness(a)
    }
}

/// convert in hex, unless is empty or too long
fn hex_empty_long(val: &[u8]) -> String {
    if val.is_empty() {
        "<empty>".to_owned()
    } else if val.len() > 2000 {
        let len = val.len();

        format!(
            "{}...truncated, original size is {} bytes...{}",
            val[0..128].to_lower_hex_string(),
            len,
            val[len - 128..len].to_lower_hex_string()
        )
    } else {
        val.to_lower_hex_string()
    }
}
