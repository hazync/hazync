//! hzengine: the Hazync proving engine's host on OpenVM 2.1 (see ../../ENGINE.md).
//!
//! One SDK, one VM config, one guest image. The SDK is self-recursive (DeferralAggProver::verify_stark): a proof of
//! the image can verify other proofs of the same image, including ones that already verified others.
//!
//! Files in the key directory (`--dir`):
//!   sdk.pk         cached proving keys (app + aggregation + deferral)
//!   openvm.toml    the VM config the guest is built against
//!   agg.vk         aggregation verifying key (what every proof verifies against)
//!   guest.vmexe    the built guest
//!   baseline.json  the guest's verification baseline (program + initial memory commitments)
//!   image.json     the guest's (exe, vm) commitments as the guest sees them, hex
//! A proof `P` is two files: P.proof.json (VersionedVmStarkProof) and P.journal (its journal bytes).
mod block;
mod capture;
mod engine;
mod r0;
mod selftest;
mod serve;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use eyre::Result;

#[derive(Parser)]
#[command(version, about = "Hazync proving engine (OpenVM 2.1)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build every proving key and write the VM config.
    Keygen {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Build the guest against the keys; write the executable, its baseline and its image commitments.
    Build {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        guest: PathBuf,
    },
    /// M1 self-test: prove a leaf committing `value`.
    SelftestLeaf {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        value: u64,
        #[arg(long)]
        out: PathBuf,
    },
    /// M1 self-test: prove a node verifying two proofs of this image.
    SelftestNode {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        left: PathBuf,
        #[arg(long)]
        right: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// M1 negative control: a node over tampered inputs must be refused in-guest (execute only).
    SelftestNegative {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        left: PathBuf,
        #[arg(long)]
        right: PathBuf,
        #[arg(long)]
        tamper: String,
    },
    /// Prove Hazync's guest from a RISC Zero input stream; `--child` proofs are verified in-guest, in env::verify order.
    Prove {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        child: Vec<PathBuf>,
        /// Put this image's id at bytes 4..36 of the stream (where the product host writes its METHOD_ID).
        #[arg(long)]
        self_id: bool,
        #[arg(long)]
        out: PathBuf,
    },
    /// Execute Hazync's guest on a stream (no proof): journal, cost, instructions.
    Execute {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        child: Vec<PathBuf>,
        #[arg(long)]
        self_id: bool,
        #[arg(long)]
        journal: Option<PathBuf>,
        /// The guest's whole stdout (collect lines) goes here.
        #[arg(long)]
        log: Option<PathBuf>,
    },
    /// A whole block from `engine-export`: chunks, trie, and the aggregate verifying them all in-guest.
    ProveBlock {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        export: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Also verify every chunk/trie proof on its own (the aggregate verifies them anyway).
        #[arg(long)]
        verify_each: bool,
    },
    /// Long-lived engine for the product host's prover backend: JSON requests on stdin, replies on stdout.
    Serve {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Verify a proof: STARK against agg.vk + baseline, its public values against its journal, and its image.
    Verify {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        proof: PathBuf,
    },
}

fn main() -> Result<()> {
    openvm_stark_sdk::utils::setup_tracing();
    match Cli::parse().cmd {
        Cmd::Keygen { dir } => engine::keygen(&dir),
        Cmd::Build { dir, guest } => engine::build(&dir, &guest),
        Cmd::SelftestLeaf { dir, value, out } => selftest::leaf(&dir, value, &out),
        Cmd::SelftestNode { dir, left, right, out } => selftest::node(&dir, &left, &right, &out),
        Cmd::SelftestNegative { dir, left, right, tamper } => selftest::negative(&dir, &left, &right, &tamper),
        Cmd::Prove { dir, input, child, self_id, out } => r0::prove(&dir, &input, &child, self_id, &out),
        Cmd::Execute { dir, input, child, self_id, journal, log } => r0::execute(&dir, &input, &child, self_id, journal.as_deref(), log.as_deref()),
        Cmd::ProveBlock { dir, export, out, verify_each } => block::prove_block(&dir, &export, &out, verify_each),
        Cmd::Serve { dir } => serve::serve(&dir),
        Cmd::Verify { dir, proof } => engine::verify(&dir, &proof),
    }
}
