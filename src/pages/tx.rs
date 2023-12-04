use std::str::from_utf8;

use bitcoin::{
    blockdata::script::Instruction,
    consensus::{encode::serialize_hex, serialize},
    Address, Amount, BlockHash, Denomination, OutPoint, Script, ScriptBuf, Transaction, TxOut,
};
use bitcoin_private::hex::exts::DisplayHex;
use maud::{html, Markup};

use crate::{
    error::Error,
    network,
    pages::size_rows,
    render::{self, AmountRow, Html, Plural},
    req::ParsedRequest,
    rpc::headers::HeightTime,
    state::BlockTemplate,
    threads::update_mempool_info::{TxidWeightFee, WeightFee},
    NetworkExt,
};

use super::html_page;

pub const IO_PER_PAGE: usize = 10;

pub fn page(
    tx: &Transaction,
    height_time: Option<(BlockHash, HeightTime)>,
    prevouts: &[TxOut],
    output_spent_height: Vec<Option<u32>>,
    page: usize,
    mempool_fees: BlockTemplate,
    parsed: &ParsedRequest,
    user_provided: bool,
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

    let sum_outputs: u64 = tx.output.iter().map(|o| o.value).sum();
    let sum_inputs: u64 = prevouts.iter().map(|o| o.value).sum();
    let fee = sum_inputs.saturating_sub(sum_outputs); // saturating never happens on confirmed/mempool-accepted tx, but we show also user made txs

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
                let amount = amount_str(previous_output.value);
                let previous_script_pubkey = (previous_output.value != u64::MAX)
                    .then(|| previous_output.script_pubkey.clone());
                let previous_script_pubkey_type = script_type(&previous_output.script_pubkey);
                let script_sig = (!input.script_sig.is_empty()).then(|| input.script_sig.clone());
                let witness = input.witness.clone();

                let p2wsh_witness_script = previous_script_pubkey
                    .as_ref()
                    .map(|s| s.is_v0_p2wsh())
                    .unwrap_or(false)
                    .then(|| witness.last().map(|e| ScriptBuf::from(e.to_vec())))
                    .flatten();

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
                    p2wsh_witness_script,
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
        .zip(
            output_spent_height
                .into_iter()
                .skip(output_start)
                .take(IO_PER_PAGE),
        )
        .map(|((i, output), spent_height)| {
            let address = Address::from_script(&output.script_pubkey, network()).ok();

            let output_link = if let Some(spent_height) = spent_height {
                let n = network().as_url_path();
                Some(format!("{n}o/{txid}:{i}/{spent_height}"))
            } else {
                None
            };
            let amount = amount_str(output.value);
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

    let hex = if tx.size() > 1_000 {
        let bytes = serialize(&tx);
        html! {
            (&bytes[..500].to_lower_hex_string())
            b { "...truncated, original size " (tx.size()) " bytes..." }
            (&bytes[tx.size()-500..].to_lower_hex_string())

        }
    } else {
        html! { (serialize_hex(&tx)) }
    };

    let wf = WeightFee {
        weight: tx.weight(),
        fee: fee as usize,
    };

    let content = html! {

        section {
            hgroup {
                h1 { "Transaction" }
                p { (render::Txid::from((txid, false))) }
            }

            table class="striped" {
                tbody {
                    (block_link)
                    @if !tx.is_coin_base() && !prevouts.iter().any(|p| p.value == u64::MAX) {
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
                        @if let Some((i, outpoint, amount, link, previous_script_pubkey, previous_script_pubkey_type, script_sig, witness, p2wsh_witness_script, sequence)) = val {

                            tr id=(format!("i{i}")) {
                                th class="row-index" {
                                    (i)
                                }

                                td {
                                    @if !parsed.response_type.is_text() {
                                        br;
                                    }

                                    div {
                                        "Previous outpoint"
                                        p { (outpoint.html()) }
                                    }

                                    @if let Some(previous_script_pubkey) = previous_script_pubkey {
                                        div {
                                            "Previous script pubkey"
                                            @if let Some(previous_script_pubkey_type) = previous_script_pubkey_type {
                                                 " (" (previous_script_pubkey_type) ")"
                                            }
                                        }

                                        p {  (previous_script_pubkey.html()) }
                                    }

                                    div { "Sequence"}
                                    p { code { (sequence) } }

                                    @if let Some(script_sig) = script_sig {
                                        div { "Script sig"}
                                        p { (script_sig.html()) }
                                    }
                                    @if !witness.is_empty() {
                                        div { "Witness"}
                                        p { (witness.html()) }
                                    }
                                    @if let Some(p2wsh_witness_script) = p2wsh_witness_script {
                                        div { "P2wsh witness script"}
                                        p { (p2wsh_witness_script.html()) }
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
                    @for (i, address, amount, output_link, script_pubkey, script_type, op_return_string) in outputs {
                        tr id=(format!("o{i}")) {
                            th class="row-index" {
                                (i)
                            }
                            td {
                                @if !parsed.response_type.is_text() {
                                    br;
                                }
                                @if let Some(address) = address {
                                    div {
                                        "Address"
                                        p { (address.html()) }
                                    }
                                }
                                div {
                                    "Script pubkey"
                                    @if let Some(script_type) = script_type {
                                        " (" (script_type) ")"
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
                                    a data-tooltip="Spent" href=(output_link) { (amount) }
                                } @else {
                                    @if script_pubkey.is_provably_unspendable() {
                                        em data-tooltip="Provably unspendable" style="font-style: normal" { (amount) }
                                    } @else {
                                        em data-tooltip="Unspent" style="font-style: normal" { (amount) }
                                    }

                                }
                            }
                        }
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
                    (size_rows(tx.size(), tx.weight().to_wu() as usize))
                    tr {
                        th { "Version" }
                        td class="right" { (tx.version) }
                    }
                    tr {
                        th { "Lock time" }
                        td class="right" { (tx.lock_time.to_consensus_u32()) }
                    }
                }
            }

            h2 id="hex" { "Hex "}

            code { (hex) }

        }
    };

    Ok(html_page("Transaction", content, parsed))
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
            th { "Fee rate (BTC/KvB)" }
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
