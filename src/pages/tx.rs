use std::str::from_utf8;

use bitcoin::{
    blockdata::script::Instruction,
    consensus::{encode::serialize_hex, serialize},
    Address, Amount, BlockHash, Denomination, OutPoint, Script, Transaction, TxOut,
};
use bitcoin_hashes::hex::ToHex;
use maud::{html, Markup};

use crate::{
    error::Error, network, pages::size_rows, render::Html, rpc::headers::HeightTime, NetworkExt,
};

use super::html_page;

pub const IO_PER_PAGE: usize = 10;

pub fn page(
    tx: &Transaction,
    height_time: Option<(BlockHash, HeightTime)>,
    prevout: &[TxOut],
    page: usize,
) -> Result<Markup, Error> {
    let txid = tx.txid();
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
    let separator_input = (prev_input.is_some() && next_input.is_some()).then(|| " | ");

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
    let separator_output = (prev_output.is_some() && next_output.is_some()).then(|| " | ");

    let sum_outputs: u64 = tx.output.iter().map(|o| o.value).sum();
    let sum_inputs: u64 = prevout.iter().map(|o| o.value).sum();
    let fee = sum_inputs - sum_outputs;

    let inputs = tx
        .input
        .iter()
        .skip(input_start)
        .take(IO_PER_PAGE)
        .zip(prevout.iter())
        .enumerate()
        .map(|(i, (input, previous_output))| {
            let po = &input.previous_output;
            if po == &OutPoint::null() {
                None
            } else {
                let link = format!("{}t/{}", network().as_url_path(), po.txid);
                let amount = amount_str(previous_output.value);
                let previous_script_pubkey = previous_output.script_pubkey.clone();
                let previous_script_pubkey_type = script_type(&previous_output.script_pubkey);
                let script_sig = (!input.script_sig.is_empty()).then(|| input.script_sig.clone());

                // The following logic makes hex the witness elements, empty elements become "<empty>".
                // Moreover there is a deduplication logic where same consucutive elements like "00 00"
                // are shown as "00 2 times". This helps showing tx like
                // 73be398c4bdc43709db7398106609eea2a7841aaf3a4fa2000dc18184faa2a7e which contains
                // 500_001 empty push
                let mut witness = vec![];
                let mut count = 1;
                let w = input.witness.to_vec();
                let mut iter = w.into_iter();
                if let Some(mut before) = iter.next() {
                    let mut last = None;
                    for current in iter {
                        if before != current {
                            if count == 1 {
                                witness.push(hex_empty_long(&before));
                            } else {
                                witness.push(format!(
                                    "{} {} times",
                                    hex_empty_long(&before),
                                    count
                                ));
                            }
                            count = 1;
                        } else {
                            count += 1;
                        }

                        last = Some(current.clone());
                        before = current;
                    }
                    if let Some(last) = last {
                        if count == 1 {
                            witness.push(hex_empty_long(&last));
                        } else {
                            witness.push(format!("{} {} times", hex_empty_long(&last), count));
                        }
                    }
                }

                let sequence = format!("0x{:x}", input.sequence);
                Some((
                    i + input_start,
                    po,
                    amount,
                    link,
                    previous_script_pubkey,
                    previous_script_pubkey_type,
                    script_sig,
                    witness,
                    sequence,
                ))
            }
        });

    let outputs = tx
        .output
        .iter()
        .skip(output_start)
        .take(IO_PER_PAGE)
        .enumerate()
        .map(|(i, output)| {
            let address = Address::from_script(&output.script_pubkey, network()).ok();

            let output_link = if output.script_pubkey.is_provably_unspendable() {
                None
            } else {
                Some(format!("{}o/{}/{}", network().as_url_path(), txid, i))
            };
            let amount = amount_str(output.value);
            let script_pubkey = output.script_pubkey.clone();
            let script_type = script_type(&output.script_pubkey);

            let op_return_string = output
                .script_pubkey
                .is_op_return()
                .then(|| {
                    for instruction in output.script_pubkey.instructions() {
                        if let Ok(Instruction::PushBytes(data)) = instruction {
                            return from_utf8(&data).ok();
                        }
                    }
                    None
                })
                .flatten();

            (
                i + output_start,
                address,
                amount,
                output_link,
                script_pubkey,
                script_type,
                op_return_string,
            )
        });

    let inputs_plural = if tx.input.len() > 1 { "s" } else { "" };
    let outputs_plural = if tx.output.len() > 1 { "s" } else { "" };

    let block_link = if let Some((block_hash, height_time)) = height_time {
        html! {
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
                td class="right red" { "Unconfirmed" }
            }
        }
    };

    let hex = if tx.size() > 1_000 {
        let bytes = serialize(&tx);
        html! {
            (&bytes[..500].to_hex())
            b { "...truncated, original size " (tx.size()) " bytes..." }
            (&bytes[tx.size()-500..].to_hex())

        }
    } else {
        html! { (serialize_hex(&tx)) }
    };

    let content = html! {

        section {
            hgroup {
                h1 { "Transaction" }
                p { (txid.html()) }
            }

            table role="grid" {
                tbody {
                    (block_link)
                    @if !tx.is_coin_base() {
                        (fee_rows(fee, tx.weight()))
                    }
                }
            }

            hgroup {
                h2 id="inputs" { (tx.input.len()) " input" (inputs_plural) }
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

            table role="grid" {
                tbody {
                    @for val in inputs {
                        @if let Some((i, outpoint, amount, link, previous_script_pubkey, previous_script_pubkey_type, script_sig, witness, sequence)) = val {

                            tr id=(format!("i{i}")) {
                                th class="row-index" {
                                    (i)
                                }


                                td {
                                    br;
                                    div {
                                        "Previous outpoint"
                                        p { (outpoint.html()) }
                                    }
                                    div {
                                        "Previous script pubkey"
                                        @if let Some(previous_script_pubkey_type) = previous_script_pubkey_type {
                                            small { " (" (previous_script_pubkey_type) ")" }
                                        }
                                    }
                                    p {  (previous_script_pubkey.html()) }

                                    div { "Sequence"}
                                    p { code { small { (sequence) }  } }

                                    @if let Some(script_sig) = script_sig {
                                        div { "Script sig"}
                                        p { (script_sig.html()) }
                                    }
                                    @if !witness.is_empty() {
                                        div { "Witness"}
                                        p {
                                            code {
                                                small {
                                                    @for (i, el) in witness.iter().enumerate()  {
                                                        @if i % 2 == 0 {
                                                            b { (el) " " }
                                                        } @else {
                                                            i { (el) " " }
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                    }

                                }
                                td class="number" {
                                    a href=(link) { (amount) }
                                }
                            }
                        }
                        @else {
                            td { "Coinbase" }
                            td {}

                        }
                    }
                }
                @if let Some(next) = next_input {
                    tfoot {
                        fr {
                            th { }
                            td { a href=(next) { "other inputs" } }
                            td { }
                        }
                    }
                }
            }

            hgroup {
                h2 id="outputs"  { (tx.output.len()) " output" (outputs_plural) }
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
            table role="grid" {
                tbody {
                    @for (i, address, amount, output_link, script_pubkey, script_type, op_return_string) in outputs {
                        tr id=(format!("o{i}")) {
                            th class="row-index" {
                                (i)
                            }
                            td {
                                br;
                                @if let Some(address) = address {
                                    div {
                                        "Address"
                                        p { (address.html()) }
                                    }
                                }
                                div {
                                    "Script pubkey"
                                    @if let Some(script_type) = script_type {
                                        small { " (" (script_type) ")" }
                                    }
                                }
                                p { (script_pubkey.html()) }

                                @if let Some(op_return_string) = op_return_string {
                                    div { "Op return in utf8" }
                                    p { code { (op_return_string) } }
                                }
                            }
                            td class="number" {
                                @if let Some(output_link) = output_link {
                                    a href=(output_link) { (amount) }
                                } @else {
                                    em data-tooltip="Provably unspendable" style="font-style: normal" { (amount) }
                                }
                            }
                        }
                    }
                }
                @if let Some(next) = next_output {
                    tfoot {
                        fr {
                            th { }
                            td { a href=(next) { "other outputs" } }
                            td { }
                        }
                    }
                }
            }

            h2 id="details" { "Details "}
            table role="grid" {
                tbody {
                    (size_rows(tx.size(), tx.weight()))
                    tr {
                        th { "Version" }
                        td class="right" { (tx.version) }
                    }
                    tr {
                        th { "Lock time" }
                        td class="right" { (tx.lock_time.to_u32()) }
                    }
                }
            }

            h2 id="hex" { "Hex "}

            small { code { (hex) } }

        }
    };

    Ok(html_page("Transaction", content))
}

/// convert in hex, unless is empty or too long
fn hex_empty_long(val: &[u8]) -> String {
    if val.is_empty() {
        "<empty>".to_owned()
    } else if val.len() > 2000 {
        let len = val.len();

        format!(
            "{}...truncated, original size is {} bytes...{}",
            val[0..128].to_hex(),
            len,
            val[len - 128..len].to_hex()
        )
    } else {
        val.to_hex()
    }
}

fn amount_str(val: u64) -> String {
    format!(
        "{:.8}",
        Amount::from_sat(val).to_float_in(Denomination::Bitcoin)
    )
}

pub fn fee_rows(fee: u64, weight: usize) -> Markup {
    let virtual_size = weight as f64 / 4.0;
    let rate_sat_vb = fee as f64 / virtual_size;
    let rate_sat_vb = format!("{:.1} sat/vB", rate_sat_vb);

    let rate_btc_kvb = (fee as f64 / 100_000_000.0) / (virtual_size / 1_000.0); //  BTC/KvB
    let rate_btc_kvb = format!("{:.8}", rate_btc_kvb);

    html! {
        tr {
            th { "Fee" }
            td class="number" { (amount_str(fee))  }
        }

        tr {
            th { "Fee rate (BTC/KvB)" }
            td class="number" { em data-tooltip=(rate_sat_vb) style="font-style: normal" { (rate_btc_kvb) } }
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
    } else if script.is_v0_p2wpkh() {
        "v0 p2wpkh"
    } else if script.is_v0_p2wsh() {
        "v0 p2wsh"
    } else if script.is_v1_p2tr() {
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
