//! A narrated run of the chat history: append, page back, search, reopen.

use chat_history::{ChatHistory, Error, Role};

fn show(turns: &[chat_history::Turn]) {
    for t in turns {
        let who = if t.role == Role::User { "user" } else { "assistant" };
        println!("  {who:<9} {}", t.text);
    }
}

fn main() -> Result<(), Error> {
    let dir = std::env::temp_dir().join(format!("kerno-example-chat-history-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let chat = ChatHistory::open(&dir)?;
    let s42 = chat.session("support-42")?;
    let s43 = chat.session("support-43")?;
    let script = [
        (Role::User, "My nightly backup did not run."),
        (Role::Assistant, "Which time zone is the server in?"),
        (Role::User, "UTC. The backup is set for 02:00."),
        (Role::Assistant, "Then the job ran before the disk was mounted."),
        (Role::User, "How do I delay it?"),
        (Role::Assistant, "Move the backup to 03:00 in the schedule."),
        (Role::User, "Done, thanks."),
    ];
    for (role, text) in script {
        chat.append(s42, role, text)?;
    }
    chat.append(s43, Role::User, "Is the backup encrypted?")?;
    chat.append(s43, Role::Assistant, "Yes, with the key in the vault.")?;

    chat.close()?;
    let chat = ChatHistory::open(&dir)?;
    assert_eq!(chat.session("support-42")?, s42, "the session survives a reopen");

    let mut before = None;
    let mut sizes = Vec::new();
    while sizes.len() < 10 {
        let page = chat.recent(s42, 3, before)?;
        println!("page {}:", sizes.len() + 1);
        show(&page.turns);
        sizes.push(page.turns.len());
        before = page.older;
        if before.is_none() {
            break;
        }
    }
    assert_eq!(sizes, [3, 3, 1]);

    let hits = chat.search(s42, "backup", 10)?;
    println!("search \"backup\" in support-42:");
    show(&hits);
    let mut texts: Vec<&str> = hits.iter().map(|t| t.text.as_str()).collect();
    texts.sort_unstable();
    assert_eq!(
        texts,
        [
            "Move the backup to 03:00 in the schedule.",
            "My nightly backup did not run.",
            "UTC. The backup is set for 02:00."
        ]
    );
    println!("sessions keep their turns apart: true");

    chat.close()?;
    std::fs::remove_dir_all(&dir).expect("remove the demo database");
    Ok(())
}
