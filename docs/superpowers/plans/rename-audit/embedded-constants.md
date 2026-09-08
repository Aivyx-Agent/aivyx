# Embedded product-identity string constants — audit

Scope: `crates/` tree only (not `docs/`), per Task 1 brief. Grep strategy:
broad `grep -rn '"aivyx"' crates --include="*.rs"` (and case-varied /
pattern-varied follow-ups: `.join("aivyx")`, `.join(".aivyx")`,
`"AIVYX_`, `"aivyx.toml"`, `"Aivyx`, `systemd`/`launchd`/`.service`/
`.plist`), then each hit's real context read to classify as (a) genuine
product-identity string in scope for this rename, or (b) not in scope
(internal crate name, unrelated/arbitrary test fixture, sibling-repo
reference, or historical/stale doc comment).

**Headline surprise, read this first:** the brief anticipated a short,
contained list (keyring name, socket path, systemd unit, env prefix,
config filename, log path — plus a config-path-construction site "likely
in `aivyx-config`"). The real picture is much bigger in two ways Tasks 2+
need to plan for:

1. **The `~/.aivyx/` dotdir convention is not confined to `aivyx-config`.**
   It's independently re-implemented (via `dirs::home_dir().join(".aivyx")…`,
   not a shared helper) as the base directory for OAuth/tool-process config
   across **13 separate crates** (every productivity integration —
   `aivyx-gmail`, `aivyx-calendar`, `aivyx-drive`, `aivyx-contacts`,
   `aivyx-notion`, `aivyx-obsidian`, `aivyx-n8n`, `aivyx-toolkit`,
   `aivyx-auth-cli`, `aivyx-google-oauth`, `aivyx-tool`, plus
   `aivyx-cli`'s `connect`/`connect_kitchen`/`doctor`/`pack` modules and
   `verticals/aivyx-kitchen-toolkit`), **31 real code call sites** total
   (Table B below) — not "a couple of calls in aivyx-config."
2. **There's a second, entirely separate identity axis this task's brief
   didn't ask about but the code itself says is coupled to the product
   name: the assistant's own default *persona* name.**
   `aivyx-config/src/lib.rs`'s `DEFAULT_ASSISTANT_NAME` constant is
   `"Aivyx"`, and its own doc comment states outright: *"Matches the
   product name — operators who don't care about renaming get 'Aivyx' by
   default."* That one constant fans out through ~50 further literal
   `"Aivyx"` occurrences across `aivyx-channel`, `aivyx-cli`,
   `aivyx-config`, `aivyx-core`, `aivyx-desktop`, `aivyx-ipc`, and
   `aivyx-web` — default notification titles, the compiled-in system
   prompt ("You are Aivyx, a capable assistant…"), the desktop app's
   window title/tray tooltip/autostart name, the web Studio's page title
   and wordmark, an HTTP Basic-auth realm string, generated TOML content,
   and more (Table F). This grounding task's own authority stopped at
   *cataloging* these literals — deciding whether the assistant's own
   spoken/display name renames alongside the product was a real
   product-shape call outside this task's scope. **Resolved by the
   operator after this report was written: yes, it renames too** — the
   assistant's default persona name becomes `"Aivyx PA"`, matching the
   product name exactly as the original doc comment already argued,
   still fully overridable by an operator's own `[profile]
   assistant_name`. Table F below reflects that resolution.

Table columns: `file:line | current literal | what it's for | new literal`.
"New literal" is filled in for every row — either because the design spec
(`docs/superpowers/specs/2026-09-08-aivyx-pa-rename-design.md`) already
locked the answer unambiguously (binary/config-path identifiers →
`aivyx-pa` family), or because the operator resolved Table F's assistant-
name question directly (→ `"Aivyx PA"`, except the two sandbox-dir sites,
which follow the technical kebab-case identifier instead — see their own
rows above).

---

## Table A — Previously confirmed (keyring service name)

| file:line | current literal | what it's for | new literal |
|---|---|---|---|
| `crates/aivyx-channel/src/keyring_store.rs:23` | `const SERVICE: &str = "aivyx";` | OS keyring (Secret Service/Keychain/Credential Manager) service name under which the master passphrase is stored | `"aivyx-pa"` |

---

## Table B — `~/.aivyx/` / `~/.config/aivyx/` / XDG dir-segment construction (real code, non-test)

All of these build a path by joining the literal `"aivyx"` (XDG-style,
under `.config`/`.local/share`) or `".aivyx"` (bare dotdir under `$HOME`)
as a directory-name segment. Two conventions coexist in the codebase today
(see "Note on `.aivyx` vs `aivyx`" below) — both are in scope, both
presumably move to `aivyx-pa`/`.aivyx-pa` together.

| file:line | current literal | what it's for | new literal |
|---|---|---|---|
| `crates/aivyx-config/src/lib.rs:5967` | `home.join(".aivyx").join("workspace")` | default agent workspace dir (`[workspace] path` fallback) | `.aivyx-pa/workspace` |
| `crates/aivyx-config/src/lib.rs:5994` | `xdg.join("aivyx").join("store.redb")` | default encrypted store path via `$XDG_DATA_HOME` | `aivyx-pa/store.redb` |
| `crates/aivyx-config/src/lib.rs:6000-6001` | `.join("aivyx").join("store.redb")` | default encrypted store path via `$HOME/.local/share` fallback | `aivyx-pa/store.redb` |
| `crates/aivyx-config/src/lib.rs:5834` | `home.join("aivyx-sandbox")` | default `fs_root` sandbox dir (`$HOME/aivyx-sandbox`) | `aivyx-pa-sandbox` — resolved: already hyphenated, not the `.aivyx` dotdir family, so it follows the technical kebab-case identifier, not the prose product name |
| `crates/aivyx-cli/src/bin/aivyx_modules/init.rs:1569` | `format!("{home}/aivyx-sandbox")` | init wizard's mirrored copy of the same default (comment admits it must be kept in sync by hand) | `aivyx-pa-sandbox` — same as above |
| `crates/aivyx-ipc/src/protocol.rs:36` | `PathBuf::from(xdg).join("aivyx").join("daemon.sock")` | daemon Unix socket path via `$XDG_RUNTIME_DIR` (preferred) | `aivyx-pa/daemon.sock` |
| `crates/aivyx-ipc/src/protocol.rs:42-43` | `.join("aivyx").join("daemon.sock")` | daemon Unix socket path via `$HOME/.local/share` fallback | `aivyx-pa/daemon.sock` |
| `crates/aivyx-channel/src/mcp_status.rs:49` | `base.join("aivyx").join("mcp-status.json")` | shared MCP-server status snapshot, beside the store | `aivyx-pa/mcp-status.json` |
| `crates/aivyx-cli/src/bin/aivyx_modules/init_templates.rs:120` | `PathBuf::from(xdg).join("aivyx").join("templates")` | user override dir for `aivyx init --template` | `aivyx-pa/templates` |
| `crates/aivyx-cli/src/bin/aivyx_modules/init_templates.rs:131` | `.join("aivyx").join("templates")` | same, `$HOME/.local/share` fallback | `aivyx-pa/templates` |
| `crates/aivyx-cli/src/bin/aivyx.rs:5695` | `directories::ProjectDirs::from("", "", "aivyx")` | `ProjectDirs` qualifier/org/app triple for the kvcache store default | `("", "", "aivyx-pa")` |
| `crates/aivyx-cli/src/bin/aivyx.rs:5697` | `std::env::temp_dir().join("aivyx").join("kvcache")` | kvcache dir fallback when `ProjectDirs` unavailable | `aivyx-pa/kvcache` |
| `crates/aivyx-cli/src/bin/aivyx.rs:8006` | `.join(".aivyx")` | (context: workspace-adjacent path in a CLI module; same dotdir family) | `.aivyx-pa` |
| `crates/aivyx-cli/src/bin/aivyx_modules/connect.rs:424` | `home.join(".config").join("aivyx").join("aivyx.toml")` | `find_aivyx_toml`'s XDG fallback lookup for the config file | `.config/aivyx-pa/aivyx-pa.toml` |
| `crates/aivyx-cli/src/bin/aivyx_modules/connect.rs:87` | `home.join(".aivyx").join("tool-processes").join(self.key)` | per-tool-process OAuth config dir, generic (all `aivyx connect <service>` targets) | `.aivyx-pa/tool-processes/<key>` |
| `crates/aivyx-cli/src/bin/aivyx_modules/connect_kitchen.rs:31` | `home.join(".aivyx").join("tool-processes").join("kitchen")` | kitchen vertical's tool-process config dir | `.aivyx-pa/tool-processes/kitchen` |
| `crates/aivyx-cli/src/bin/aivyx_modules/doctor.rs:404` | `.join(".aivyx")` | `aivyx doctor`'s kitchen config-path check | `.aivyx-pa` |
| `crates/aivyx-cli/src/bin/aivyx_modules/pack.rs:103` | `.join(".aivyx/packs")` | vertical-pack unpack destination (`~/.aivyx/packs/<name>/<version>/`) | `.aivyx-pa/packs` |
| `crates/aivyx-auth-cli/src/config_file.rs:58` | `.join(".aivyx")` | shared `aivyx-auth-cli` OAuth config-dir helper | `.aivyx-pa` |
| `crates/aivyx-calendar/src/auth_cli/config_file.rs:51` | `.join(".aivyx")` | calendar tool-process config dir | `.aivyx-pa` |
| `crates/aivyx-calendar/src/oauth/storage.rs:18` | `.join(".aivyx")` | calendar OAuth token storage dir | `.aivyx-pa` |
| `crates/aivyx-contacts/src/auth_cli/config_file.rs:51` | `.join(".aivyx")` | contacts tool-process config dir | `.aivyx-pa` |
| `crates/aivyx-contacts/src/lib.rs:80` | `.join(".aivyx")` | contacts OAuth token storage dir | `.aivyx-pa` |
| `crates/aivyx-drive/src/auth_cli/config_file.rs:51` | `.join(".aivyx")` | drive tool-process config dir | `.aivyx-pa` |
| `crates/aivyx-drive/src/lib.rs:106` | `.join(".aivyx")` | drive OAuth token storage dir | `.aivyx-pa` |
| `crates/aivyx-gmail/src/auth_cli/config_file.rs:51` | `.join(".aivyx")` | gmail tool-process config dir | `.aivyx-pa` |
| `crates/aivyx-gmail/src/oauth/storage.rs:23` | `.join(".aivyx")` | gmail OAuth token storage dir | `.aivyx-pa` |
| `crates/aivyx-google-oauth/src/storage.rs:78` | `.join(".aivyx")` | shared Google OAuth storage helper (backs gmail/calendar/drive/contacts) | `.aivyx-pa` |
| `crates/aivyx-n8n/src/lib.rs:47` | `.join(".aivyx")` | n8n tool-process config dir | `.aivyx-pa` |
| `crates/aivyx-notion/src/lib.rs:87` | `.join(".aivyx")` | notion tool-process config dir | `.aivyx-pa` |
| `crates/aivyx-obsidian/src/lib.rs:52` | `.join(".aivyx")` | obsidian tool-process config dir | `.aivyx-pa` |
| `crates/aivyx-toolkit/src/config.rs:83` | `.join(".aivyx")` | toolkit tool-process config-file path | `.aivyx-pa` |
| `crates/aivyx-toolkit/src/config.rs:96` | `.join(".aivyx")` | toolkit tool-process dir (budget/health/task stores live beside it) | `.aivyx-pa` |
| `crates/verticals/aivyx-kitchen-toolkit/src/config.rs:70` | `.join(".aivyx")` | kitchen-toolkit vertical's own config path (same pattern, private vertical) | `.aivyx-pa` |

Test mirror (not a separate real default, but breaks if the above moves
without it): `crates/aivyx-config/src/tests.rs:11509` asserts
`PathBuf::from(home).join(".aivyx").join("workspace")` against the real
default computed in `lib.rs:5967`.

### Note on `.aivyx` vs `aivyx` — two coexisting conventions

The dotdir form `~/.aivyx/…` (workspace, tool-process OAuth configs,
packs) and the XDG form `~/.config/aivyx/…` / `~/.local/share/aivyx/…`
(daemon socket, store, templates, mcp-status, `aivyx.toml` itself) are
**both real and both independently in use** — this isn't drift to fix as
part of the rename, just something Task 2 needs to carry forward
faithfully as two parallel families (`.aivyx-pa` and
`.config/aivyx-pa`/`.local/share/aivyx-pa`), not collapse into one.

---

## Table C — Default config filename

| file:line | current literal | what it's for | new literal |
|---|---|---|---|
| `crates/aivyx-cli/src/bin/aivyx_modules/init.rs:21` | `const CONFIG_FILE: &str = "aivyx.toml";` | the file `aivyx init` writes and every config loader reads by default | `"aivyx-pa.toml"` (decide exact spelling in Task 2 — `aivyx-pa.toml` follows the binary-rename convention most literally) |
| `crates/aivyx-cli/src/bin/aivyx_modules/connect.rs:424` (see Table B) | `"aivyx.toml"` path segment | XDG-fallback config file lookup | matches whatever Task 2 picks above |
| `crates/aivyx-config/src/tests.rs:4247` | `let toml_path = tmp.path().join("aivyx.toml");` | test fixture reflecting the real default filename | mirror of the above |

Generated-TOML / prose mentions of `aivyx.toml` inside doc comments and
`eprintln!`/wizard text (e.g. `aivyx-config/src/lib.rs` — dozens of doc
comments; `aivyx-cli/.../init.rs` wizard prompts; `daemon_service.rs`'s
`cwd.join("aivyx.toml").exists()` check) are downstream of the same
constant and not enumerated individually here — they're mechanical once
`CONFIG_FILE`'s value changes, but there are enough of them that Task 2
should budget real time for the sweep, not treat it as a single-line fix.
**Corrected during review**: `grep -rn '"aivyx\.toml"' crates --include="*.rs"
| wc -l` genuinely returns **228 lines**, not this row's original "~90"
estimate — budget for the real, larger number.

---

## Table D — systemd/launchd service identity (Chapter Anchor)

All defined in one file, `crates/aivyx-cli/src/bin/aivyx_modules/daemon_service.rs`:

| file:line | current literal | what it's for | new literal |
|---|---|---|---|
| `daemon_service.rs:66` | `pub const SERVICE_UNIT: &str = "aivyx-daemon.service";` | systemd **user** unit filename (`~/.config/systemd/user/aivyx-daemon.service`) | `"aivyx-pa-daemon.service"` |
| `daemon_service.rs:68` | `pub const LAUNCHD_LABEL: &str = "com.aivyx.daemon";` | macOS LaunchAgent label + plist filename (`~/Library/LaunchAgents/com.aivyx.daemon.plist`) | `"com.aivyx-pa.daemon"` (or `com.aivyx.pa.daemon` — reverse-DNS-style casing decision for Task 2) |
| `daemon_service.rs:72` | `pub const ENV_FILE_REL: &str = "aivyx/daemon.env";` | passphrase env-file path relative to the systemd config dir (`~/.config/aivyx/daemon.env`) | `"aivyx-pa/daemon.env"` |
| `daemon_service.rs:17` (doc comment) | `~/Library/LaunchAgents/com.aivyx.daemon.plist` | describes the above | mirrors `LAUNCHD_LABEL` |
| `daemon_service.rs:389` | `"logs: log show --predicate 'process == \"aivyx\"'{}"` | operator-facing hint text for finding launchd logs — the predicate must match the **actual binary name** | `"aivyx-pa"` (must track the `[[bin]] name` rename) |
| `daemon_service.rs:247-248` | `"status: systemctl --user status aivyx-daemon\n  logs: journalctl --user -u aivyx-daemon -f"` | operator-facing hint text, derived from `SERVICE_UNIT`'s basename minus `.service` | mirrors `SERVICE_UNIT` |
| Test mirrors: `daemon_service.rs:530-620` (`systemd_unit_has_the_load_bearing_directives`, `launchd_plist_*` tests) | assorted `/home/u/.config/aivyx/…`, `com.aivyx.daemon`, `/b/aivyx` literals | assert the rendered unit/plist byte-for-byte | must move in lockstep with the consts above, not enumerated per-line here |

`sensitive_paths.rs` (Ward/Portcullis) also has two literal test fixtures
mirroring these real paths — `/home/alice/.config/aivyx/daemon.env` and
`/home/alice/.local/share/aivyx/store.redb` at lines 258-259 — same
"must move in lockstep, not independently interesting" status.

---

## Table E — `AIVYX_*` environment variable prefix

The prefix itself (`AIVYX_`) is the literal identity, per brief step 1's
framing ("any `AIVYX_*`-prefixed env var whose prefix is the literal
identity, not crate-internal"). Every one of the ~30 distinct env vars
below genuinely uses that prefix — none are crate-internal-namespaced
(e.g. there is no `AIVYX_CORE_*` family). Definitions (the authoritative
source of the literal) are almost all in one file; usages beyond the
`const` declaration itself (largely in test files calling
`env.set("AIVYX_ROLE", …)` etc.) run into the hundreds of lines across
`aivyx-config/src/tests.rs` alone and are not enumerated individually —
treat every `AIVYX_`-prefixed literal in the tree as downstream of the
list below and needing the same prefix swap.

| file:line | current literal | what it's for |
|---|---|---|
| `crates/aivyx-config/src/lib.rs:5646` | `AIVYX_MODEL` | model override |
| `crates/aivyx-config/src/lib.rs:5647` | `AIVYX_SYSTEM_PROMPT` | system prompt override |
| `crates/aivyx-config/src/lib.rs:5648` | `AIVYX_FS_ROOT` | fs sandbox root override |
| `crates/aivyx-config/src/lib.rs:5650` | `AIVYX_WORKSPACE` | workspace path override |
| `crates/aivyx-config/src/lib.rs:5651` | `AIVYX_STORAGE_PATH` | encrypted store path override |
| `crates/aivyx-config/src/lib.rs:5654` | `AIVYX_MEMORY_MAX_PER_TOPIC` | memory retention override |
| `crates/aivyx-config/src/lib.rs:5655` | `AIVYX_MEMORY_TTL_SECS` | memory retention override |
| `crates/aivyx-config/src/lib.rs:5656` | `AIVYX_PASSPHRASE` | master passphrase (also `crates/aivyx-channel/src/passphrase.rs:67`'s `DEFAULT_ENV_VAR`, and referenced in `daemon_service.rs`'s rendered systemd `EnvironmentFile`) |
| `crates/aivyx-config/src/lib.rs:5657` | `AIVYX_TELEGRAM_TOKEN` | Telegram channel token |
| `crates/aivyx-config/src/lib.rs:5658` | `AIVYX_TELEGRAM_CHAT_ID` | Telegram chat id |
| `crates/aivyx-config/src/lib.rs:5662` | `AIVYX_DISCORD_TOKEN` | Discord channel token |
| `crates/aivyx-config/src/lib.rs:5663` | `AIVYX_DISCORD_APPLICATION_ID` | Discord app id |
| `crates/aivyx-config/src/lib.rs:5668` | `AIVYX_SLACK_BOT_TOKEN` | Slack bot token |
| `crates/aivyx-config/src/lib.rs:5669` | `AIVYX_SLACK_APP_TOKEN` | Slack app token |
| `crates/aivyx-config/src/lib.rs:5670` | `AIVYX_SLACK_TEAM_ID` | Slack team id |
| `crates/aivyx-config/src/lib.rs:5674` | `AIVYX_ROLE` | active-role override |
| `crates/aivyx-config/src/lib.rs:5675` | `AIVYX_OPENAI_API_KEY` | OpenAI-compatible key override |
| `crates/aivyx-config/src/lib.rs:5676` | `AIVYX_OPENAI_BASE_URL` | OpenAI-compatible base URL override |
| `crates/aivyx-config/src/lib.rs:5677` | `AIVYX_KVCACHE_STORE_PATH` | kvcache store path override |
| `crates/aivyx-config/src/lib.rs:5678` | `AIVYX_PROVIDER` | LLM provider selection |
| `crates/aivyx-config/src/lib.rs:5682` | `AIVYX_EMBEDDING_API_KEY` | embeddings API key override |
| `crates/aivyx-calendar/src/calendar_client.rs:116` | `AIVYX_CALENDAR_CACHE_TTL_SECS` | calendar cache TTL |
| `crates/aivyx-cli/src/bin/aivyx_modules/connect_kitchen.rs:133,139,145` | `AIVYX_KITCHEN_BASE_URL`, `AIVYX_KITCHEN_API_KEY`, `AIVYX_KITCHEN_ORGANIZATION_ID` | kitchen vertical connect flow |
| `crates/aivyx-desktop/src/gate_watch.rs:32,60,83` / `main.rs:43` | `AIVYX_STUDIO_URL`, `AIVYX_STUDIO_TOKEN` | desktop shell's daemon/Studio discovery |
| `crates/aivyx-desktop/src/main.rs:91` | `AIVYX_BIN` | override for which binary the desktop shell shells out to (defaults to literal `"aivyx"` — see Table F) |
| `crates/aivyx-llm/src/anthropic/provider.rs:95` | `AIVYX_ANTHROPIC_PDF_PAGE_CAP` | Anthropic PDF page-cap override |
| `crates/aivyx-pack/src/lib.rs:42` | `env!("AIVYX_BUILD_TARGET")` | **build-time** env var baked in via `build.rs`/Cargo — different mechanism than the runtime `std::env::var` ones above; verify the producing side (likely a `build.rs` outside `crates/*/src`) before renaming |
| `crates/aivyx-mcp/tests/mcp_bridge_e2e.rs:267,332` | `AIVYX_MCP_SANDBOX_PROBE`, `AIVYX_MCP_ENV_PROBE` | test-only sandbox probes — real prefix, but only ever set inside the test itself, never read by production code outside the test's own subprocess; low priority, cosmetic-only if left unrenamed |
| `crates/aivyx-tool/tests/proxy_e2e.rs:492` | `AIVYX_SANDBOX_PROBE` | same as above, test-only |
| `crates/aivyx-core/src/tools/shell.rs:1130,1143` | `AIVYX_TEST_SECRET_CANARY` | test-only canary for secret-leak detection, not a real product env var — **category (b), do not rename**: the point of the test is that *some* env var leaked, the name itself is arbitrary |

---

## Table F — Assistant persona identity (resolved: renames to "Aivyx PA")

Root: `crates/aivyx-config/src/lib.rs:365-368`:
```rust
/// Default assistant name used by [`Profile`] when no `[profile]
/// assistant_name` is declared in TOML. Matches the product name —
/// operators who don't care about renaming get "Aivyx" by default;
/// operators who want a named assistant override it explicitly. Q5(b)
/// resolution at Phase 57 sign-off (PRODUCT.md P13 commit 5).
pub const DEFAULT_ASSISTANT_NAME: &str = "Aivyx";
```
and, separately, `crates/aivyx-config/src/lib.rs:217` — the compiled-in
system prompt the model actually sees:
```rust
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are Aivyx, a capable assistant running locally on the operator's own machine. \
...
```

| file:line | current literal | what it's for | new literal |
|---|---|---|---|
| `aivyx-config/src/lib.rs:368` | `DEFAULT_ASSISTANT_NAME = "Aivyx"` | default persona name (`[profile] assistant_name` fallback) — doc comment explicitly says it "matches the product name" | `Aivyx PA` |
| `aivyx-config/src/lib.rs:217` | `"You are Aivyx, a capable assistant…"` (`DEFAULT_SYSTEM_PROMPT`) | the actual text sent to the LLM every turn | `Aivyx PA` |
| `aivyx-cli/src/bin/aivyx_modules/identity.rs:134` | `if bundle.profile.assistant_name != "Aivyx"` | identity-import diff check — **hardcoded duplicate of `DEFAULT_ASSISTANT_NAME` instead of referencing the constant**, worth fixing while touching this line anyway | `Aivyx PA` |
| `aivyx-cli/.../init.rs:1377,1459` | `unwrap_or("Aivyx")` | init wizard's assistant-name default | `Aivyx PA` |
| `aivyx-cli/.../profile.rs:326,544,622,844,850,889,933` | `"Aivyx"` (7 sites) | `aivyx profile` command: display default, generated TOML, skill-proposer hint value | `Aivyx PA` |
| `aivyx-cli/.../role.rs:323,360` | `"Aivyx"` | role-scoped profile generation | `Aivyx PA` |
| `aivyx-cli/.../toml_edit_apply.rs:414,547,562,606,626` | `"Aivyx"` | applying a profile-hint proposal to TOML | `Aivyx PA` |
| `aivyx-config/src/config_write.rs:252,1877,1883,2053,2061` | `"Aivyx"` | config-section rewriter, doc example + tests | `Aivyx PA` |
| `aivyx-channel/src/profile_prompt.rs:40` (doc comment) | `assistant_name = "Aivyx"` | describes the synthesized default | mirrors const |
| `aivyx-channel/src/daemon_server.rs:8634` (test) | `assert_eq!(summary.assistant_name, "Aivyx")` | daemon status summary default | mirrors const |
| `aivyx-channel/src/identity_export.rs:318` | `"assistant_name": "Aivyx"` | exported identity-bundle JSON default | `Aivyx PA` |
| `aivyx-channel/src/notify_dispatcher.rs:672,680` (test) | `"Aivyx"` | notification title default | mirrors const |
| `aivyx-channel/src/notify_email.rs:241-247,334,342,376` | `"Aivyx notification"` | fallback email subject when none given | `Aivyx PA` |
| `aivyx-channel/src/notify_webui.rs:162-164,188,231,251` | `"Aivyx"` | fallback web-UI notification title | `Aivyx PA` |
| `aivyx-channel/src/reflection_scheduler.rs:1521` | `format!("Aivyx — proactive ({:?})", …)` | proactive-surfacing notification subject | `Aivyx PA` |
| `aivyx-channel/src/skill_auto_proposer.rs:2256,2567,2576` | `"Aivyx"` | skill-proposer's own assistant-name-change proposal machinery | `Aivyx PA` |
| `aivyx-channel/src/web_ui.rs:472` | `"WWW-Authenticate: Basic realm=\"Aivyx Studio\"\r\n"` | HTTP Basic-auth realm string shown by browsers on the web UI's login prompt | `Aivyx PA` |
| `aivyx-ipc/src/protocol.rs:558,3660` | `"Aivyx"` | wire-protocol default + doc comment | `Aivyx PA` |
| `aivyx-audit/src/lib.rs:2142,2150` | `"Aivyx"` | audit-record test fixture for an assistant-name mutation | mirrors const |
| `aivyx-core/src/skill_proposer/judge.rs:1004,1025,1174,1181,1435,1460` | `"Aivyx"` | skill-proposal judge test fixtures | mirrors const |
| `aivyx-desktop/src/main.rs:82` | `.set_app_name("Aivyx")` | OS autostart entry's registered app name | `Aivyx PA` |
| `aivyx-desktop/src/main.rs:223` | `.with_title("Aivyx Studio")` | native window title | `Aivyx PA` |
| `aivyx-desktop/src/main.rs:254` | `.with_tooltip("Aivyx")` | system-tray icon tooltip | `Aivyx PA` |
| `aivyx-desktop/src/main.rs:~250` | `MenuItem::new("Quit Aivyx", …)` | tray menu item text | `Aivyx PA` |
| `aivyx-desktop/src/gate_watch.rs:149` | `.summary("Aivyx — approval needed")` | desktop OS notification summary | `Aivyx PA` |
| `aivyx-web/src/main.rs:1163` | `document::Title { "Aivyx Studio" }` | web Studio browser tab title | `Aivyx PA` |
| `aivyx-web/src/main.rs:1414` | `img { alt: "Aivyx" }` | logo image alt text | `Aivyx PA` |
| `aivyx-web/src/main.rs:1415` | `span { "AIVYX" }` (wordmark) | sidebar brand wordmark text | `Aivyx PA` — noted during review: the current literal is all-caps as a literal Rust string, not via a CSS `text-transform` rule (`stitch.css` confirmed to have none), so this row is a deliberate casing call, not a copy-paste of the standard prose convention; `AIVYX PA` (matching the existing all-caps visual style) is equally defensible if that reads better in the actual UI — Task 2's own judgment call |
| `aivyx-web/src/main.rs:7074` | `placeholder: "Aivyx (default)"` | assistant-name input placeholder | `Aivyx PA` |
| `aivyx-tui/examples/connect.rs:185` | `"Aivyx never sees a shared secret — you own the app."` | example-file prose describing the OAuth model | `Aivyx PA` — arguably closer to docs prose than a runtime constant, but lives in `crates/` |

### Resolved by the operator (was an open question for Task 2)

The design spec (`docs/superpowers/specs/2026-09-08-aivyx-pa-rename-design.md`)
locks: *"'Aivyx PA' becomes the flagship product's own name everywhere it
is currently used as the product."* It did not explicitly say whether
the **assistant's own spoken/display persona name** — what it calls
itself in a notification, a window title, or its own system prompt — was
"the product" for this purpose, or a distinct, renameable-independently
identity axis (the way "Siri" is a persona name distinct from "iOS" the
product). Task 1 deliberately left this a product-shape decision rather
than guessing. **The operator resolved it directly: yes, `DEFAULT_ASSISTANT_NAME`
becomes `"Aivyx PA"`**, matching the doc comment's own original argument
("matches the product name") — still fully overridable per-operator via
`[profile] assistant_name` at Profile-seed time, same as today. Task 2
applies this single decision consistently across all ~35 sites in this
table.

---

## Table G — MCP client self-identification

| file:line | current literal | what it's for | new literal |
|---|---|---|---|
| `crates/aivyx-mcp/src/transport.rs:169` | `name: "aivyx".into()` | the `clientInfo.name` field sent in the MCP `initialize` handshake to every MCP server this daemon connects to | `"aivyx-pa"` (decide in Task 2 — external MCP servers may log/display this) |

---

## Table H — Outbound webhook notification "source" field

| file:line | current literal | what it's for | new literal |
|---|---|---|---|
| `crates/aivyx-channel/src/notify_webhook.rs:89` | `source: "aivyx"` | the `"source"` field in the JSON payload POSTed to a configured webhook target | `"aivyx-pa"` (decide in Task 2 — this is a wire-visible field an external receiver may match on) |
| `notify_webhook.rs:13` (doc comment), `:254`, `:278` (tests) | mirrors of the above | | mirrors decision above |

---

## Table I — Binary name literal (invoking `aivyx` as a subprocess/command)

| file:line | current literal | what it's for | new literal |
|---|---|---|---|
| `crates/aivyx-desktop/src/main.rs:91` | `std::env::var("AIVYX_BIN").unwrap_or_else(\|_\| "aivyx".to_string())` | which binary the desktop shell shells out to, to drive the daemon | `"aivyx-pa"` |
| `crates/aivyx-cli/src/bin/aivyx_modules/init.rs:929` | `command = "aivyx"` inside generated `[[mcp_server]]` TOML (bundled web-search server invocation) | the generated config tells the daemon to re-invoke its own binary as an MCP server subprocess | `"aivyx-pa"` |
| `crates/aivyx-config/src/tests.rs:4247` (test, mirrors above pattern) | `command = "aivyx"` | test fixture for the same bundled-MCP-server TOML shape | mirrors decision above |
| `crates/aivyx-cli/src/bin/aivyx.rs:514` | `println!("aivyx {}", env!("CARGO_PKG_VERSION"))` | `aivyx --version` output | `"aivyx-pa"` |
| `crates/aivyx-cli/src/bin/aivyx.rs:489,1119,1395,1398,1416,5136` | `eprintln!("aivyx: …")` / `eprintln!("aivyx daemon: …")` / `eprintln!("aivyx config sources:")` | CLI's own error/status prefix convention on stderr | `"aivyx-pa"` throughout (6+ sites in this one file; a straightforward but non-trivial find-and-replace since some are inside format strings with interpolation) |
| `crates/aivyx-cli/src/bin/aivyx_modules/init.rs:2407-2434` | `eprintln!("  aivyx  — chat with your agent…")` etc. | first-run wizard's "next steps" help text, several lines each naming the CLI invocation | `"aivyx-pa"` throughout |

Not exhaustive for this category — `aivyx daemon install`, `aivyx doctor`,
`aivyx connect`, etc. appear as command-name prose throughout
`aivyx_modules/*.rs` in `--help` text, error messages, and doc comments.
A rough `grep -rn '"aivyx ' crates/aivyx-cli --include="*.rs" | wc -l`
(literal `"aivyx ` — the binary name followed by a space, inside a string
literal, i.e. instances of the CLI naming itself in user-facing text)
returns **~140 lines**. This is the single largest mechanical-rename
surface in the whole audit and deserves its own pass/checklist in
whichever task actually does the binary rename, rather than being treated
as folded into Table E's environment-variable work.

---

## Table J — `[aivyx]` TOML config section (the passphrase section)

| file:line | current literal | what it's for | new literal |
|---|---|---|---|
| `crates/aivyx-config/src/lib.rs:3958` | `aivyx: RawAivyx` field on the outer raw-TOML struct | makes `[aivyx]` a real TOML table name (serde derives the section name from the field name) | `aivyx_pa: RawAivyxPa` (Rust field/struct → snake_case, following normal Rust naming, not the prose "Aivyx PA" convention used elsewhere in this document) → the TOML section operators write becomes `[aivyx_pa]`; this is a breaking config-format change, not just an internal rename, so treat with the same care as the CLI/env-var rename |
| `crates/aivyx-config/src/lib.rs:5599-5602` | `struct RawAivyx { passphrase: Option<String> }` | backing struct | `struct RawAivyxPa { passphrase: Option<String> }` — same as above |
| `crates/aivyx-channel/src/keyring_store.rs:6-7` (doc comment) | `` `[aivyx] passphrase]` in the TOML `` | describes the same section | `` `[aivyx_pa] passphrase]` `` — mirrors decision above |

---

## Category (b) — matched the grep, out of scope, no change

| file:line | literal | why excluded |
|---|---|---|
| `crates/aivyx-core/src/relevance/keywords.rs:189` | `"aivyx"` in `extract_keywords("...https://github.com/aivyx", 10)` | arbitrary test input (a GitHub-URL-shaped string used to test punctuation handling); not asserting anything about product identity |
| `crates/aivyx-obsidian/src/tools/search.rs:453` | `"aivyx"` in `frontmatter_matches(fm, "tags", "aivyx")` | generic substring-match test using `"project-aivyx"` as arbitrary example tag content |
| `crates/aivyx-cli/src/bin/aivyx_modules/pack.rs:248` | `publisher = "Aivyx"` | test-only manifest fixture (`#[cfg(test)] mod tests`); `publisher` is free-text set by whoever builds a pack, not a compiled-in default |
| `crates/aivyx-pack/src/lib.rs:486` | `publisher = "Aivyx Test"` | same, test fixture |
| `crates/aivyx-core/src/tools/git.rs:1197` | `run(&["config", "user.name", "Aivyx Test"])` | arbitrary test git-identity value |
| `crates/aivyx-drive/src/tools/list_drives.rs:196,206,28` | `"name": "Aivyx Working Group"` | arbitrary example Drive resource name in a test/doc example |
| daemon_service.rs doc comment line 12 mentioning `~/Library/LaunchAgents/com.aivyx.daemon.plist` etc. | (covered by Table D already; listed there, not duplicated) | — |
| `crates/aivyx-config/src/lib.rs:5` doc comment: *"Before this crate landed, `crates/aivyx-channel/src/bin/aivyx.rs`…"* | historical reference to a file path | **verified this path does not currently exist** (`crates/aivyx-channel/src/bin/` is absent) — it's describing pre-refactor history, not a live path; nothing to rename |
| Sibling-repo mentions in doc comments (e.g. `aivyx-coder`, `aivyx-confine`, `aivyx-capability` internal crate names throughout) | crate names / other repos | explicitly out of scope per the design spec ("the 34 internal Cargo crate names… stay exactly as they are") |

---

## Summary counts

- Table A (keyring): 1 site.
- Table B (`.aivyx`/`aivyx` dir-segment construction, real code): 31 sites across 23 files in 15 crates + 1 test mirror.
- Table C (`aivyx.toml` filename): 1 canonical const + 228 downstream literal mentions (not individually enumerated — corrected during review from an original "~90" estimate).
- Table D (systemd/launchd): 3 consts + operator-facing hint strings + test mirrors, all in 1 file (plus 2 test-fixture mirrors in `sensitive_paths.rs`).
- Table E (`AIVYX_*` env prefix): ~30 distinct env var names, definitions concentrated in `aivyx-config/src/lib.rs` with a handful in `aivyx-calendar`, `aivyx-cli`, `aivyx-desktop`, `aivyx-llm`, `aivyx-pack`; hundreds of downstream test-literal usages not enumerated.
- **Table F (assistant persona identity — the surprise cluster): ~35 sites across 11 crates, all downstream of one product-shape question — resolved by the operator: renames to "Aivyx PA."**
- Table G (MCP client name): 1 site.
- Table H (webhook source field): 1 site + 3 mirrors.
- Table I (binary name in CLI's own user-facing text): 6+ enumerated sites, 110 lines matching the broader pattern (confirmed by independent review), not individually enumerated.
- Table J (`[aivyx]` TOML section): 3 sites — a breaking config-format change (Rust field/struct → `aivyx_pa`/`RawAivyxPa`, TOML section → `[aivyx_pa]`), not just an internal rename; corrected during review from an initial mis-applied "Aivyx PA" prose substitution.
- Category (b): 8 hits confirmed genuinely out of scope.
