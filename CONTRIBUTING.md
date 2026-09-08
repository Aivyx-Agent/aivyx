# Contributing to Aivyx PA

Thanks for your interest in contributing. Aivyx PA is a single-operator
personal-agent platform by design (PRODUCT.md P1 + P6), but well-shaped
contributions — new channels, tools, and provider adapters that fit the existing
SDK surfaces — are welcome.

Before your first pull request, please read the one thing that is **required**:
the **Contributor License Agreement** in the next section. Everything else here
is the normal "how to work in this repo" guidance.

## 1. The Contributor License Agreement (required)

Aivyx PA is **source-available under [BUSL-1.1](LICENSE)** and is offered under a
dual model — **free for personal/non-commercial use, paid for commercial use**
(see [`COMMERCIAL.md`](COMMERCIAL.md)). For that model to be lawful, the project
must hold the right to license **all** of the code — including your
contributions — under both the BUSL-1.1 terms and separate commercial terms.

A plain "inbound = outbound" contribution does **not** give the project the right
to commercially sublicense your code. So, **before any contribution can be
merged, you must agree to the [Contributor License Agreement (`CLA.md`)](CLA.md).**
In short, the CLA: you keep your copyright, and you grant the Licensor a broad,
irrevocable license to use, relicense, and **commercially sublicense** your
contributions. Read [`CLA.md`](CLA.md) for the exact terms — it is short.

**How you agree:** every commit in your pull request must carry a
`Signed-off-by` trailer matching the author, added automatically by committing
with `-s`:

```sh
git commit -s -m "your message"
```

By signing off you certify the [Developer Certificate of Origin](#developer-certificate-of-origin)
**and** accept the [CLA](CLA.md) for that contribution. A maintainer cannot merge
a PR whose commits are not signed off. If you are contributing on behalf of an
employer, make sure you have their permission first (the CLA covers this).

> **Why this exists:** without it, the relicense and the commercial offering
> could not legally cover contributed lines. This gate must precede any external
> PR — see [`docs/LICENSING.md`](docs/LICENSING.md) §3.

## 2. Before you open a PR

1. **File an issue first** for anything beyond a small fix — describe the shape
   and let's agree on the approach before you build it.
2. **Architectural changes** that touch `DESIGN.md` or `PRODUCT.md` require a
   formal amendment under `docs/amendments/` (the process is established —
   thirteen have been filed). Open the discussion in the issue.
3. New channels / tools / adapters should fit the existing SDK surfaces
   (`docs/CHANNEL_SDK.md`, `docs/TOOL_SDK.md`).

## 3. Building & testing

Install the pre-commit hook once per clone, then run the full sweep before
pushing:

```sh
# Pre-commit hook (recommended once per clone)
./scripts/install-hooks.sh

# Full sweep — the workspace holds at zero warnings
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Python conformance suites (no daemon required)
python3 -m unittest discover examples/python-channel/tests
python3 -m unittest discover examples/python-tool/tests
```

The pre-commit hook runs `cargo clippy --workspace --all-targets -- -D warnings`
before every commit. PRs are expected to be clippy-clean and to keep
`cargo test --workspace` green.

## 4. Pull-request checklist

- [ ] Every commit is **signed off** (`git commit -s`) — see §1.
- [ ] You have read and accept [`CLA.md`](CLA.md).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- [ ] `cargo test --workspace` passes.
- [ ] Any `DESIGN.md` / `PRODUCT.md` change has an amendment under
      `docs/amendments/`.

---

## Developer Certificate of Origin

By making a contribution to this project, you certify the
[Developer Certificate of Origin 1.1](https://developercertificate.org/):

> 1. The contribution was created in whole or in part by you and you have the
>    right to submit it under the open source license indicated in the file; or
> 2. The contribution is based upon previous work that, to the best of your
>    knowledge, is covered under an appropriate open source license and you have
>    the right under that license to submit that work with modifications,
>    whether created in whole or in part by you, under the same license (unless
>    you are permitted to submit under a different license), as indicated in the
>    file; or
> 3. The contribution was provided directly to you by some other person who
>    certified (1), (2) or (3) and you have not modified it.
> 4. You understand and agree that this project and the contribution are public
>    and that a record of the contribution (including all personal information
>    you submit with it, including your sign-off) is maintained indefinitely and
>    may be redistributed consistent with this project and the requirements
>    stated above.

Your `Signed-off-by` line certifies the DOC above **and** accepts the
[CLA](CLA.md) for the signed contribution.
