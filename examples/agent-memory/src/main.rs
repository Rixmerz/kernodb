//! Command-line front end of the agent memory.
//!
//! ```text
//! agent-memory <dir> remember <text> [--tag <tag>]...   store a memory with its vector
//! agent-memory <dir> list                               every memory, oldest first
//! agent-memory <dir> recent [n]                         the n newest memories
//! agent-memory <dir> between <from_us> <to_us>          memories created in a time window
//! agent-memory <dir> recall <text> [k]                  the k nearest by vector
//! agent-memory <dir> show <id>                          one memory and the state of its vector
//! agent-memory <dir> correct <id> <text>                a new revision; the vector goes stale
//! agent-memory <dir> rebuild <id>                       recompute a stale vector
//! agent-memory <dir> history <id>                       the retained revisions of a memory
//! agent-memory <dir> forget <id>                        delete a memory, its vector and summaries
//! agent-memory <dir> prune                              drop all but the newest revision of every unit
//! agent-memory <dir> reclaim                            compact the live files (no promise of erasure)
//! agent-memory <dir> summarize <id>...                  summarise memories
//! agent-memory <dir> summaries                          list summaries
//! agent-memory <dir> refresh                            rebuild stale summaries
//! agent-memory demo <dir>                               run the whole scenario
//! ```

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use agent_memory::{AgentMemory, Embedder, Error, Memory, Result, VectorStatus};
use kerno_units::SystemClock;

fn usage() -> ExitCode {
    eprintln!("usage: agent-memory <dir> <command> [args]   (see the README for the commands)");
    ExitCode::from(2)
}

fn show(m: &Memory) {
    println!("#{} rev {} [{}] at {}us: {}", m.id, m.revision, m.tags.join(","), m.created_at.0, m.text);
}

fn parse_id(s: &str) -> Result<u64> {
    s.trim_start_matches('#').parse().map_err(|_| Error::Invalid(format!("{s:?} is not a memory number")))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [demo, dir] if demo == "demo" => run_demo(PathBuf::from(dir)),
        [dir, cmd, rest @ ..] => run(PathBuf::from(dir), cmd, rest),
        _ => return usage(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open(dir: PathBuf) -> Result<AgentMemory> {
    let (mem, report) = AgentMemory::open(&dir, Arc::new(SystemClock), Embedder::default())?;
    if report.vectors_demoted > 0 || report.findings > 0 {
        eprintln!("note: {} vector(s) served stale, {} finding(s)", report.vectors_demoted, report.findings);
    }
    Ok(mem)
}

fn run(dir: PathBuf, cmd: &str, rest: &[String]) -> Result<()> {
    let mem = open(dir)?;
    let embedder = Embedder::default();
    let out = match (cmd, rest) {
        ("remember", [text, flags @ ..]) => {
            let mut tags = Vec::new();
            let mut it = flags.iter();
            while let Some(f) = it.next() {
                match (f.as_str(), it.next()) {
                    ("--tag", Some(t)) => tags.push(t.clone()),
                    _ => return Err(Error::Invalid("expected --tag <tag>".into())),
                }
            }
            let id = mem.remember(text, &tags, Some(embedder.embed(text.as_bytes())))?;
            println!("stored #{id}");
            Ok(())
        }
        ("list", []) => mem.all().map(|all| all.iter().for_each(show)),
        ("recent", []) => mem.recent(5).map(|all| all.iter().for_each(show)),
        ("recent", [n]) => {
            let n = n.parse().map_err(|_| Error::Invalid("n is not a number".into()))?;
            mem.recent(n).map(|all| all.iter().for_each(show))
        }
        ("between", [from, to]) => {
            let t =
                |s: &String| s.parse().map(kerno_units::Timestamp).map_err(|_| Error::Invalid("not a number".into()));
            mem.recall_between(t(from)?, t(to)?).map(|all| all.iter().for_each(show))
        }
        ("recall", [text, k @ ..]) => {
            let k = k.first().map_or(Ok(3), |k| k.parse()).map_err(|_| Error::Invalid("k is not a number".into()))?;
            mem.recall_similar(&embedder.embed(text.as_bytes()), k).map(|hits| {
                for h in hits {
                    print!("{:+.3}  ", h.similarity);
                    show(&h.memory);
                }
            })
        }
        ("show", [id]) => {
            let id = parse_id(id)?;
            show(&mem.get(id)?);
            println!("vector: {:?}", mem.vector_status(id)?);
            Ok(())
        }
        ("correct", [id, text]) => {
            let id = parse_id(id)?;
            let rev = mem.correct(id, text)?;
            println!("#{id} is now revision {rev}; vector: {:?}", mem.vector_status(id)?);
            Ok(())
        }
        ("rebuild", [id]) => {
            let id = parse_id(id)?;
            mem.rebuild_vector(id)?;
            println!("#{id} vector: {:?}", mem.vector_status(id)?);
            Ok(())
        }
        ("history", [id]) => mem.history(parse_id(id)?).map(|all| all.iter().for_each(show)),
        ("prune", []) => {
            println!("{} revision(s) pruned", mem.prune_history()?);
            Ok(())
        }
        ("reclaim", []) => {
            mem.reclaim()?;
            println!("live files compacted; nothing is promised about the storage medium");
            Ok(())
        }
        ("forget", [id]) => {
            mem.forget(parse_id(id)?)?;
            println!("forgotten");
            Ok(())
        }
        ("summarize", ids) if !ids.is_empty() => {
            let ids = ids.iter().map(|i| parse_id(i)).collect::<Result<Vec<_>>>()?;
            println!("summary {}", mem.summarize(&ids)?);
            Ok(())
        }
        ("summaries", []) => {
            print_summaries(&mem);
            Ok(())
        }
        ("refresh", []) => {
            println!("{} summary(ies) rebuilt", mem.refresh_summaries()?);
            Ok(())
        }
        _ => {
            eprintln!("unknown command or arguments: {cmd}");
            Err(Error::Invalid("see the README for the commands".into()))
        }
    };
    mem.close()?;
    out
}

fn print_summaries(mem: &AgentMemory) {
    for s in mem.summaries() {
        println!(
            "summary {} over {:?}: {:?} {}",
            s.index,
            s.sources,
            s.state,
            s.text.unwrap_or_default().replace('\n', " | ")
        );
    }
}

fn run_demo(dir: PathBuf) -> Result<()> {
    let embedder = Embedder::default();
    let (mem, _) = AgentMemory::open(&dir, Arc::new(SystemClock), embedder)?;
    println!("== store memories, each with a vector the stand-in model made");
    let texts = [
        "the deploy script needs the staging token",
        "alice prefers short answers in the morning",
        "the staging token rotates every friday",
        "bob asked about the quarterly report",
    ];
    let ids: Vec<u64> = texts
        .iter()
        .map(|t| mem.remember(t, &["demo".to_owned()], Some(embedder.embed(t.as_bytes()))))
        .collect::<Result<_>>()?;
    mem.all()?.iter().for_each(show);

    println!("\n== recall by vector: \"staging token\"");
    for h in mem.recall_similar(&embedder.embed(b"staging token"), 2)? {
        print!("{:+.3}  ", h.similarity);
        show(&h.memory);
    }
    println!("\n== recall by time: the two newest");
    mem.recent(2)?.iter().for_each(show);

    println!("\n== correct #{}: the vector is reported stale and is not recalled", ids[0]);
    mem.correct(ids[0], "the deploy script needs the production token")?;
    println!("vector: {:?}", mem.vector_status(ids[0])?);
    println!(
        "memories recallable by vector: {} of {}",
        mem.recall_similar(&embedder.embed(b"production"), 9)?.len(),
        ids.len()
    );
    mem.rebuild_vector(ids[0])?;
    println!("after rebuild: {:?}", mem.vector_status(ids[0])?);
    for h in mem.recall_similar(&embedder.embed(b"deploy script production token"), 1)? {
        print!("{:+.3}  ", h.similarity);
        show(&h.memory);
    }

    println!("\n== summarise #{} and #{}", ids[0], ids[2]);
    mem.summarize(&[ids[0], ids[2]])?;
    print_summaries(&mem);
    println!("originals stay readable: #{} = {:?}", ids[0], mem.get(ids[0])?.text);

    println!("\n== forget #{}: its vector and the summary go", ids[2]);
    let vector = mem.vector_id(ids[2])?.expect("attached above");
    mem.forget(ids[2])?;
    println!("registry state of its vector: {:?}", mem.derived_state(&vector));
    println!("summaries left: {}", mem.summaries().len());
    println!("memory #{} still readable: {}", ids[0], mem.get(ids[0]).is_ok());
    println!("revisions of #{}: {}", ids[0], mem.history(ids[0])?.len());
    println!("\n== reclaim: the live files are compacted (logical removal only, no erasure promise)");
    mem.reclaim()?;

    println!("\n== close and reopen");
    mem.close()?;
    let (mem, report) = AgentMemory::open(&dir, Arc::new(SystemClock), embedder)?;
    println!("{report:?}");
    println!("#{} vector: {:?}", ids[0], mem.vector_status(ids[0])?);
    println!("memories: {}", mem.all()?.len());
    debug_assert_eq!(mem.vector_status(ids[0])?, VectorStatus::Ready);
    mem.close()
}
