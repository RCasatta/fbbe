use std::str::from_utf8;

use bitcoin::Txid;
use bitcoin::{
    blockdata::script::Instruction, consensus::encode::serialize_hex, Address, Amount, BlockHash,
    Denomination, OutPoint, Script, ScriptBuf, Transaction, TxOut,
};
use maud::{html, Markup};

use crate::{
    error::Error,
    network,
    pages::size_rows,
    render::{self, AmountRow, Html, Plural},
    req::ParsedRequest,
    rpc::headers::HeightTime,
    state::{BlockTemplate, SpendPoint},
    threads::{
        index_addresses::Height,
        update_mempool_info::{TxidWeightFee, WeightFee},
    },
    NetworkExt,
};

use super::html_page;

pub const IO_PER_PAGE: usize = 10;

pub enum OutputStatus {
    ConfirmedSpent(Height),
    UnconfirmedSpent(SpendPoint),
    Unspent,
    Unknown,
}

#[allow(clippy::too_many_arguments)] // TODO: remove
pub fn page(
    txid: Txid,
    tx: &Transaction,
    height_time: Option<(BlockHash, HeightTime)>,
    prevouts: &[TxOut], // May be sparse: non-visible inputs can be represented as TxOut::NULL
    fee: Option<u64>,
    output_status: Vec<OutputStatus>,
    page: usize,
    mempool_fees: BlockTemplate,
    parsed: &ParsedRequest,
    user_provided: bool,
    known_tx: Option<String>,
) -> Result<Markup, Error> {
    let network_url_path = network().as_url_path();
    let start = page * IO_PER_PAGE;
    if start >= tx.input.len() && start >= tx.output.len() {
        return Err(Error::InvalidPageNumber);
    }

    let last_page_input = tx.input.len().saturating_sub(1) / IO_PER_PAGE;
    let last_page_output = tx.output.len().saturating_sub(1) / IO_PER_PAGE;
    log::debug!("last page {last_page_input} {last_page_output}");

    let input_start = start.min(last_page_input * IO_PER_PAGE);
    let output_start = start.min(last_page_output * IO_PER_PAGE);
    log::debug!("from {input_start} {output_start}");

    let prev_input = (page > 0 && last_page_input != 0).then(|| {
        format!(
            "{}t/{}/{}#inputs",
            network_url_path,
            txid,
            (last_page_input - 1).min(page - 1)
        )
    });
    let next_input = (page < last_page_input)
        .then(|| format!("{}t/{}/{}#inputs", network_url_path, txid, page + 1));
    let separator_input = (prev_input.is_some() && next_input.is_some()).then_some(" | ");

    let prev_output = (page > 0 && last_page_output != 0).then(|| {
        format!(
            "{}t/{}/{}#outputs",
            network_url_path,
            txid,
            (last_page_output - 1).min(page - 1)
        )
    });
    let next_output = (page < last_page_output)
        .then(|| format!("{}t/{}/{}#outputs", network_url_path, txid, page + 1));
    let separator_output = (prev_output.is_some() && next_output.is_some()).then_some(" | ");

    let inputs = tx
        .input
        .iter()
        .skip(input_start)
        .take(IO_PER_PAGE)
        .zip(prevouts.iter().skip(input_start))
        .enumerate()
        .map(|(i, (input, previous_output))| {
            let po = &input.previous_output;
            if po == &OutPoint::null() {
                None
            } else {
                let link = format!("{}t/{}#o{}", network().as_url_path(), po.txid, po.vout);
                let amount = amount_str(previous_output.value.to_sat());
                let previous_script_pubkey = (previous_output.value.to_sat() != u64::MAX)
                    .then(|| previous_output.script_pubkey.clone());
                let previous_script_pubkey_type = script_type(&previous_output.script_pubkey);
                let script_sig = (!input.script_sig.is_empty()).then(|| input.script_sig.clone());
                let witness = input.witness.clone();

                let p2wsh_witness_script = previous_script_pubkey
                    .as_ref()
                    .map(|s| s.is_p2wsh())
                    .unwrap_or(false)
                    .then(|| witness.last().map(|e| ScriptBuf::from(e.to_vec())))
                    .flatten();

                let sequence = format!("0x{:x}", input.sequence);

                let mut fields = vec![("Previous outpoint".to_string(), po.html())];
                if let Some(previous_script_pubkey) = previous_script_pubkey {
                    let label = match previous_script_pubkey_type {
                        Some(t) => format!("Previous script pubkey ({t})"),
                        None => "Previous script pubkey".to_string(),
                    };
                    fields.push((label, previous_script_pubkey.html()));
                }
                fields.push(("Sequence".to_string(), html! { code { (sequence) } }));
                if let Some(script_sig) = script_sig {
                    fields.push(("Script sig".to_string(), script_sig.html()));
                }
                if !witness.is_empty() {
                    fields.push(("Witness".to_string(), witness.html()));
                }
                if let Some(p2wsh_witness_script) = p2wsh_witness_script {
                    fields.push((
                        "P2wsh witness script".to_string(),
                        p2wsh_witness_script.html(),
                    ));
                }
                let amount = html! { a href=(link) { (amount) } };

                Some((i + input_start, fields, amount))
            }
        });

    let outputs = tx
        .output
        .iter()
        .skip(output_start)
        .take(IO_PER_PAGE)
        .enumerate()
        .zip(
            output_status
                .into_iter()
                .skip(output_start)
                .take(IO_PER_PAGE),
        )
        .map(|((i, output), spent_status)| {
            let address = Address::from_script(&output.script_pubkey, network()).ok();

            let output_link = match spent_status {
                OutputStatus::ConfirmedSpent(h) => {
                    let n = network().as_url_path();
                    Some(format!("{n}o/{txid}:{i}/{h}"))
                }
                OutputStatus::UnconfirmedSpent(s) => {
                    let n = network().as_url_path();
                    let txid = s.txid();
                    let vin = s.vin();
                    Some(format!("{n}t/{txid}#i{vin}"))
                }
                OutputStatus::Unspent => None,
                OutputStatus::Unknown => None,
            };

            let amount = amount_str(output.value.to_sat());
            let script_pubkey = output.script_pubkey.clone();
            let script_type = script_type(&output.script_pubkey);

            let op_return_string = output
                .script_pubkey
                .is_op_return()
                .then(|| {
                    for instruction in output.script_pubkey.instructions().flatten() {
                        if let Instruction::PushBytes(data) = instruction {
                            return from_utf8(data.as_bytes()).ok();
                        }
                    }
                    None
                })
                .flatten();

            let mut fields = vec![];
            if let Some(address) = address {
                fields.push(("Address".to_string(), address.html()));
            }
            let label = match script_type {
                Some(t) => format!("Script pubkey ({t})"),
                None => "Script pubkey".to_string(),
            };
            fields.push((label, script_pubkey.html()));
            if let Some(op_return_string) = op_return_string {
                fields.push((
                    "Op return in utf8".to_string(),
                    html! { code { (op_return_string) } },
                ));
            }

            let (amount, amount_text) = if let Some(output_link) = output_link {
                (
                    html! { a data-tooltip="Spent" href=(output_link) { (amount) } },
                    html! { a href=(output_link) { (amount) } " (spent)" },
                )
            } else {
                let status = if script_pubkey.is_op_return() {
                    "Provably unspendable"
                } else {
                    "Unspent"
                };
                (
                    html! { em data-tooltip=(status) style="font-style: normal" { (amount) } },
                    html! { (amount) " (" (status.to_lowercase()) ")" },
                )
            };

            (i + output_start, fields, amount, amount_text)
        });

    let inputs_plural = Plural::new("input", tx.input.len());
    let outputs_plural = Plural::new("output", tx.output.len());

    let last_in_block = if height_time.is_none() {
        mempool_fees.last_in_block
    } else {
        None
    };

    let depends_on_unconfirmed = tx
        .input
        .iter()
        .any(|i| mempool_fees.mempool.contains(&i.previous_output.txid));

    let block_link = if let Some((block_hash, height_time)) = height_time {
        html! {
            @if let Some(text) = known_tx {
                th { "Note" }
                th class="right" { (text) }
            }

            tr {
                th { "Status" }
                td class="right green" { "Confirmed" }
            }

            tr {
                th { "Timestamp" }
                td class="right" { (height_time.date_time_utc()) }
            }

            tr {
                th { "Block " (height_time.height) }
                td class="right" { (block_hash.html()) }
            }
        }
    } else {
        html! {
            tr {
                th { "Status" }
                td class="right red" {
                    @if user_provided {
                        "User provided"
                    } @else if depends_on_unconfirmed {
                        "Unconfirmed with unconfirmed inputs"
                    } @else {
                        "Unconfirmed"
                    }

                }
            }
        }
    };

    let hex_link = if user_provided {
        format!("{network_url_path}txhex/{}/hex", serialize_hex(&tx))
    } else {
        format!("{network_url_path}t/{txid}/hex")
    };

    let wf = fee.map(|fee| WeightFee {
        weight: tx.weight(),
        fee: fee as usize,
    });

    let content = html! {

        section {
            hgroup {
                h1 { "Transaction" }
                p { (render::Txid::from((txid, false))) }
            }

            table class="striped" {
                tbody {
                    (block_link)
                    @if let Some(wf) = wf {
                        (fee_rows( wf, last_in_block))
                    }
                }
            }

            hgroup {
                h2 id="inputs" { (tx.input.len()) " " (inputs_plural) }
                p {
                    @if let Some(prev) = prev_input {
                        a href=(prev) { "Prev" }
                    }
                    @if let Some(separator) = separator_input {
                        (separator)
                    }
                    @if let Some(next) = next_input.as_ref() {
                        a href=(next) { "Next" }
                    }
                }
            }

            table class="striped" {
                tbody {
                    @for val in inputs {
                        @if let Some((i, fields, amount)) = val {
                            (io_rows(parsed, format!("i{i}"), i, i != input_start, &fields, amount.clone(), amount))
                        }
                        @else {
                            td { "Coinbase" }
                            td {}

                        }
                    }
                }
                @if let Some(next) = next_input {
                    tfoot {
                        tr {
                            th { }
                            td { a href=(next) { "other inputs" } }
                            td { }
                        }
                    }
                }
            }

            hgroup {
                h2 id="outputs"  { (tx.output.len()) " " (outputs_plural) }
                p {
                    @if let Some(prev) = prev_output {
                        a href=(prev) { "Prev" }
                    }
                    @if let Some(separator) = separator_output {
                        (separator)
                    }
                    @if let Some(next) = next_output.as_ref() {
                        a href=(next) { "Next" }
                    }
                }
            }
            table class="striped" {
                tbody {
                    @for (i, fields, amount, amount_text) in outputs {
                        (io_rows(parsed, format!("o{i}"), i, i != output_start, &fields, amount, amount_text))
                    }
                }
                @if let Some(next) = next_output {
                    tfoot {
                        tr {
                            th { }
                            td { a href=(next) { "other outputs" } }
                            td { }
                        }
                    }
                }
            }

            h2 id="details" { "Details "}
            table class="striped" {
                tbody {
                    (size_rows(tx.total_size(), tx.weight().to_wu() as usize))
                    tr {
                        th { "Version" }
                        td class="right" { (tx.version) }
                    }
                    tr {
                        th { "Lock time" }
                        td class="right" { (tx.lock_time.to_consensus_u32()) }
                    }
                    tr {
                        th { "Hex" }
                        td class="right" { a href=(hex_link) { "raw transaction hex" } }
                    }
                }
            }

        }
    };

    Ok(html_page("Transaction", content, parsed))
}

/// Renders an input or output: in html the fields are stacked in a single cell, in text every
/// field has its own row, because html2text sizes a cell on the sum of its lines instead of the
/// widest, making the table much wider than needed. Without table borders, an empty row
/// before every entry but the first keeps the entries apart.
fn io_rows(
    parsed: &ParsedRequest,
    id: String,
    index: usize,
    separator: bool,
    fields: &[(String, Markup)],
    amount: Markup,
    amount_text: Markup,
) -> Markup {
    if parsed.response_type.is_text() {
        let rows = fields.len() + 1;
        html! {
            @if separator {
                tr { td { "\u{a0}" } } // an empty cell would not render the line
            }
            tr id=(id) {
                th rowspan=(rows) { (index) }
                th { "Amount (BTC)" }
                td { (amount_text) }
            }
            @for (label, value) in fields {
                tr {
                    th { (label) }
                    td { (value) }
                }
            }
        }
    } else {
        html! {
            tr id=(id) {
                th class="row-index" { (index) }
                td {
                    br;
                    @for (label, value) in fields {
                        div { (label) }
                        p { (value) }
                    }
                }
                td class="number" { (amount) }
            }
        }
    }
}

fn amount_str(val: u64) -> String {
    if val == u64::MAX {
        "Not exist".to_owned()
    } else {
        format!(
            "{:.8}",
            Amount::from_sat(val).to_float_in(Denomination::Bitcoin)
        )
    }
}

pub fn fee_rows(wf: WeightFee, last_in_block: Option<TxidWeightFee>) -> Markup {
    html! {
        (AmountRow::new_with_sat("Fee", wf.fee as u64))

        tr {
            th { "Fee rate (BTC/kvB)" }
            td class="number" { (wf) }
        }
        @if let Some(last_in_block) = last_in_block.as_ref()  {
            tr {
                th { em { "Last in block" } }
                td class="number" { (last_in_block.wf) }
            }
        }
    }
}

pub fn script_type(script: &Script) -> Option<String> {
    let kind = if script.is_p2pk() {
        "p2pk"
    } else if script.is_p2pkh() {
        "p2pkh"
    } else if script.is_p2sh() {
        "p2sh"
    } else if script.is_p2wpkh() {
        "v0 p2wpkh"
    } else if script.is_p2wsh() {
        "v0 p2wsh"
    } else if script.is_p2tr() {
        "v1 p2tr"
    } else if script.is_op_return() {
        "op return"
    } else {
        ""
    };
    if kind.is_empty() {
        None
    } else {
        Some(kind.to_string())
    }
}
