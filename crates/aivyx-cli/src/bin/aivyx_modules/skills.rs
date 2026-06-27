//! `aivyx skills teach | update | forget` — Chapter Tutor.
//!
//! The operator's **direct** skill-authoring channel: writes a skill to the
//! signed persona chain via the daemon's `AuthorSkill` IPC. This is distinct
//! from the agent's scope-gated `skills.teach` tool — the operator is the
//! authority, so it works on a **grown** chain with no agent `skills.write`
//! scope. (Listing skills is the Studio Repertoire screen; `aivyx persona`
//! shows the underlying chain.)

use std::path::Path;

use aivyx_channel::daemon_client::{author_skill, daemon_is_running};
use aivyx_channel::daemon_ipc::{default_socket_path, SkillAuthorOp};

async fn require_daemon_running(socket_path: &Path) -> Result<(), String> {
    if daemon_is_running(socket_path).await {
        return Ok(());
    }
    Err(format!(
        "aivyx skills: no daemon running on socket {} — start it first with \
         `aivyx daemon run` (or just `aivyx`)",
        socket_path.display(),
    ))
}

/// `aivyx skills teach <name> <trigger> <procedure>` — add a new skill.
pub async fn run_skills_teach(
    name: &str,
    trigger: &str,
    procedure: &str,
) -> Result<(), String> {
    if name.is_empty() || trigger.is_empty() || procedure.is_empty() {
        return Err(
            "`aivyx skills teach` needs <name> <trigger> <procedure>".into(),
        );
    }
    send(SkillAuthorOp::Teach, name, Some(trigger), Some(procedure), "taught").await
}

/// `aivyx skills update <name> [--trigger T] [--procedure P]` — change an
/// existing skill's trigger and/or procedure.
pub async fn run_skills_update(
    name: &str,
    trigger: Option<&str>,
    procedure: Option<&str>,
) -> Result<(), String> {
    if name.is_empty() {
        return Err("`aivyx skills update` needs <name>".into());
    }
    if trigger.is_none() && procedure.is_none() {
        return Err(
            "`aivyx skills update` needs at least --trigger or --procedure".into(),
        );
    }
    send(SkillAuthorOp::Update, name, trigger, procedure, "updated").await
}

/// `aivyx skills forget <name>` — remove an existing skill.
pub async fn run_skills_forget(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("`aivyx skills forget` needs <name>".into());
    }
    send(SkillAuthorOp::Forget, name, None, None, "forgot").await
}

async fn send(
    op: SkillAuthorOp,
    name: &str,
    trigger: Option<&str>,
    procedure: Option<&str>,
    verb: &str,
) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    match author_skill(&socket_path, op, name, trigger, procedure).await {
        Ok(seq) => {
            eprintln!(
                "aivyx skills: {verb} {name:?} — appended to the persona chain at seq {seq}"
            );
            Ok(())
        }
        Err(e) => Err(format!("skills {verb} failed: {e}")),
    }
}
