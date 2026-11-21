use std::fmt::Display;

use super::Html;
use crate::{globals::network, pages::tx::IO_PER_PAGE, NetworkExt};
use maud::{html, Render};

pub struct OutPoint(pub bitcoin::OutPoint);

/// Whether a [`TxIoPath`] points to an input or an output
#[derive(Clone, Copy)]
pub enum Io {
    Input,
    Output,
}

/// Path, relative to the network base, of the tx page containing the given input or output,
/// eg. `t/<txid>/1#i12`, the page is omitted when it is the first.
pub struct TxIoPath<'a>(pub &'a bitcoin::Txid, pub Io, pub usize);

impl Display for TxIoPath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let TxIoPath(txid, io, index) = self;
        write!(f, "t/{txid}")?;
        let page = index / IO_PER_PAGE;
        if page > 0 {
            write!(f, "/{page}")?;
        }
        let prefix = match io {
            Io::Input => 'i',
            Io::Output => 'o',
        };
        write!(f, "#{prefix}{index}")
    }
}

/// Absolute link to the tx page containing the given input or output
pub fn tx_io_link(txid: &bitcoin::Txid, io: Io, index: usize) -> String {
    format!("{}{}", network().as_url_path(), TxIoPath(txid, io, index))
}

impl Render for OutPoint {
    fn render(&self) -> maud::Markup {
        let link = tx_io_link(&self.0.txid, Io::Output, self.0.vout as usize);

        html! {
            a href=(link) {
                code { span class="txid" { (self.0.txid) } span { ":" (self.0.vout) } }
            }
        }
    }
}

impl Html for bitcoin::OutPoint {
    fn html(&self) -> maud::Markup {
        OutPoint(*self).render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::Hash;

    #[test]
    fn tx_io_path_has_page() {
        let txid = bitcoin::Txid::from_byte_array([1; 32]);
        assert_eq!(
            TxIoPath(&txid, Io::Input, 9).to_string(),
            format!("t/{txid}#i9")
        );
        assert_eq!(
            TxIoPath(&txid, Io::Input, 10).to_string(),
            format!("t/{txid}/1#i10")
        );
        assert_eq!(
            TxIoPath(&txid, Io::Output, 25).to_string(),
            format!("t/{txid}/2#o25")
        );
    }
}
