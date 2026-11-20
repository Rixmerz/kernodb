//! Writes the second frozen evaluation set (E9-19) to `data/derived-eval-2/`:
//! `corpus.txt`, `vectors.txt`, `queries.txt` and `MANIFEST`.
//!
//! Before writing, it runs the one check the set is allowed (T-E10-V2): the
//! nDCG@10 of the arm without signal must not exceed 0.96. If it does, nothing
//! is written and the seed in `derived_eval2` must be replaced. It measures
//! nothing else.
//!
//! The files are committed. Running this again must change nothing; the test
//! `tests/derived_eval2.rs` checks that the committed files are what it writes.
//!
//! ```text
//! cargo run --release -p kerno-testkit --example freeze_derived_eval2
//! ```

use std::path::Path;

use kerno_testkit::assoc_eval::manifest_text;
use kerno_testkit::derived_eval::{render_corpus, render_queries, render_vectors};
use kerno_testkit::derived_eval2::{SATURATION_LIMIT, SEED, arm_without_signal, generate};

fn main() -> std::io::Result<()> {
    let story = generate();
    let (overall, by_class) = arm_without_signal(&story.vectors, &story.queries);
    println!("seed {SEED}: arm without signal nDCG@10 = {overall:.4} (limit {SATURATION_LIMIT})");
    for (class, score) in by_class {
        println!("  {}: {score:.4}", class.name());
    }
    if overall > SATURATION_LIMIT {
        eprintln!("saturated: choose another seed; nothing written");
        std::process::exit(1);
    }

    let corpus = render_corpus(&story.corpus);
    let vectors = render_vectors(&story.vectors);
    let queries = render_queries(&story.queries);
    let manifest = manifest_text(&[
        ("corpus.txt", corpus.as_bytes()),
        ("vectors.txt", vectors.as_bytes()),
        ("queries.txt", queries.as_bytes()),
    ]);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/derived-eval-2");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("corpus.txt"), corpus)?;
    std::fs::write(dir.join("vectors.txt"), vectors)?;
    std::fs::write(dir.join("queries.txt"), queries)?;
    std::fs::write(dir.join("MANIFEST"), manifest)?;
    println!("wrote {}", dir.display());
    Ok(())
}
