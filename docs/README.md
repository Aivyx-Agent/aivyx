# Aivyx Docs

This directory holds **phase records** and **amendments**. It is
deliberately separate from the two contract documents in the repo
root:

- **`DESIGN.md`** is the **technical contract** — the locked
  architectural decisions ("how the agent works") that every phase
  must respect. It is edited rarely, and only through the amendment
  process described below.
- **`PRODUCT.md`** is the **product contract** — the locked
  product-shape decisions ("who the agent is for, what it commits
  to do, where the line is") that every phase must respect. It is
  the sibling of `DESIGN.md`, edited under the same amendment
  process. Drafted at the Phase 12.5 product-shape review.
- **`docs/PHASE_N.md`** are **phase journals** — working documents for
  one phase at a time. They churn freely during the phase and are
  frozen at phase exit.
- **`docs/ROADMAP.md`** is the **technical roadmap** — one-paragraph
  intents for upcoming numbered phases.
- **`docs/PRODUCT_ROADMAP.md`** is the **product roadmap** — the
  sibling of `ROADMAP.md`, listing forward product-shape milestones
  derived from `PRODUCT.md`'s commitments. Milestones are named, not
  numbered, and may span one or more technical phases.
- **`docs/amendments/`** (created when first needed) holds one file per
  contract change. An amendment is the *only* legal way to modify a
  locked `DESIGN.md` *or* `PRODUCT.md` decision from inside a later
  phase.

## Why the split?

The failure mode this layout defends against is **silent contract
drift** — a later phase quietly editing a Phase 0 decision because it
became inconvenient. With the contract in one file and the phase work
in another, any such edit shows up as a `DESIGN.md` diff in a phase
commit, which is an obvious review flag.

Rule of thumb: if you're writing something that future-you in a fresh
session needs to rely on, it goes in `DESIGN.md` (technical) or
`PRODUCT.md` (product). If it's a decision log, task list, or "why we
chose A over B today," it goes in the phase doc. If it's a one-paragraph
intent for a future phase or milestone, it goes in `ROADMAP.md` or
`PRODUCT_ROADMAP.md`.

## Cross-phase reference docs

Living documents that span multiple phases and capture patterns
learned across the phase sequence. Unlike phase journals, these are
edited freely when a new adapter or subsystem teaches us something new.

- [`ADAPTER_PATTERN.md`](ADAPTER_PATTERN.md) — future-proof checklist
  for adding a new `ChannelContext` adapter, grounded in the two
  adapters in tree (`LocalChannel`, `TelegramChannel`). Read this
  first if you're about to add a third.

## Phase status

| Phase    | Status  | Doc                        | Commit    |
|----------|---------|----------------------------|-----------|
| Phase 0  | Frozen  | [PHASE_0.md](PHASE_0.md)   | `1b4f271` |
| Phase 1  | Frozen  | [PHASE_1.md](PHASE_1.md)   | `33012be` |
| Phase 2  | Frozen  | [PHASE_2.md](PHASE_2.md)   | `2b6f876` |
| Phase 3  | Frozen  | [PHASE_3.md](PHASE_3.md)   | `fa0f4ea` |
| Phase 4  | Frozen  | [PHASE_4.md](PHASE_4.md)   | `999ce87` |
| Phase 5  | Frozen  | [PHASE_5.md](PHASE_5.md)   | `6dab2a7` |
| Phase 6  | Frozen  | [PHASE_6.md](PHASE_6.md)   | `912f022` |
| Phase 7  | Frozen  | [PHASE_7.md](PHASE_7.md)   | `8164317` |
| Phase 8  | Frozen  | [PHASE_8.md](PHASE_8.md)   | `0484606` |
| Phase 9  | Frozen  | [PHASE_9.md](PHASE_9.md)   | `7052ecc` |
| Phase 10 | Frozen  | [PHASE_10.md](PHASE_10.md) | `f8f4d28` |
| Phase 11 | Frozen  | [PHASE_11.md](PHASE_11.md) | `16422e2` |
| Phase 12 | Frozen  | [PHASE_12.md](PHASE_12.md) | `16e618c` |
| Phase 13 | Frozen  | [PHASE_13.md](PHASE_13.md) | `25a09de` |
| Phase 14 | Frozen  | [PHASE_14.md](PHASE_14.md) | `0d94d32` |
| Phase 15 | Frozen  | [PHASE_15.md](PHASE_15.md) | `06dfdfd` |
| Phase 16 | Frozen  | [PHASE_16.md](PHASE_16.md) | `1ed3f90` |
| Phase 17 | Frozen  | [PHASE_17.md](PHASE_17.md) | `277d910` |
| Phase 18 | Frozen  | [PHASE_18.md](PHASE_18.md) | `6dd4f23` |
| Phase 19 | Frozen  | [PHASE_19.md](PHASE_19.md) | `986c519` |
| Phase 20 | Frozen  | [PHASE_20.md](PHASE_20.md) | `8e77075` |
| Phase 21 | Frozen  | [PHASE_21.md](PHASE_21.md) | `05cc349` |
| Phase 22 | Frozen  | [PHASE_22.md](PHASE_22.md) | `549bc6e` |
| Phase 23 | Frozen  | [PHASE_23.md](PHASE_23.md) | `8f104e8` |
| Phase 24 | Frozen  | [PHASE_24.md](PHASE_24.md) | `84962d1` |
| Phase 25 | Frozen  | [PHASE_25.md](PHASE_25.md) | `5e4144f` |
| Phase 26 | Frozen  | [PHASE_26.md](PHASE_26.md) | `e550229` |
| Phase 27 | Frozen  | [PHASE_27.md](PHASE_27.md) | `426bcc2` |
| Phase 28 | Frozen  | [PHASE_28.md](PHASE_28.md) | `2cd41d9`  |
| Phase 29 | Frozen  | [PHASE_29.md](PHASE_29.md) | `03a804b`  |
| Phase 30 | Frozen  | [PHASE_30.md](PHASE_30.md) | `8b93180` |
| Phase 31 | Frozen  | [PHASE_31.md](PHASE_31.md) | `7f16f7a` |
| Phase 32 | Frozen  | [PHASE_32.md](PHASE_32.md) | `4aabea8` |
| Phase 33 | Frozen  | [PHASE_33.md](PHASE_33.md) | `d02dda6` |
| Phase 34 | Frozen  | [PHASE_34.md](PHASE_34.md) | `0004cf2` |
| Phase 35 | Frozen  | [PHASE_35.md](PHASE_35.md) | `5786449` |
| Phase 36 | Frozen  | [PHASE_36.md](PHASE_36.md) | `1f726f9`  |
| Phase 37 | Frozen  | [PHASE_37.md](PHASE_37.md) | `b2f3bda`  |
| Phase 38 | Frozen  | [PHASE_38.md](PHASE_38.md) | `b596207`  |
| Phase 39 | Frozen  | [PHASE_39.md](PHASE_39.md) | `6f7eaf1`  |
| Phase 40 | Frozen  | [PHASE_40.md](PHASE_40.md) | `d995afd`  |
| Phase 41 | Frozen  | [PHASE_41.md](PHASE_41.md) | `87d87c2`  |
| Phase 42 | Frozen  | [PHASE_42.md](PHASE_42.md) | `0478461`  |
| Phase 43 | Frozen  | [PHASE_43.md](PHASE_43.md) | `066e633`  |
| Phase 44 | Frozen  | [PHASE_44.md](PHASE_44.md) | `104c101`  |
| Phase 45 | Frozen  | [PHASE_45.md](PHASE_45.md) | `1ca3dc4`  |
| Phase 46 | Frozen  | [PHASE_46.md](PHASE_46.md) | `074d167`  |
| Phase 47 | Frozen  | [PHASE_47.md](PHASE_47.md) | `cc0a64f`  |
| Phase 48 | Frozen  | [PHASE_48.md](PHASE_48.md) | `7fbb534`  |
| Phase 49 | Frozen  | [PHASE_49.md](PHASE_49.md) | `8cadab8`  |
| Phase 50 | Frozen  | [PHASE_50.md](PHASE_50.md) | `0104c17`  |
| Phase 51 | Frozen  | [PHASE_51.md](PHASE_51.md) | `2503d90`  |
| Phase 52 | Frozen  | [PHASE_52.md](PHASE_52.md) | `57f6c14`  |
| Phase 53 | Skipped | (folded into Phase 54)     |            |
| Phase 54 | Frozen  | [PHASE_54.md](PHASE_54.md) | `f4e47be`  |
| Phase 55 | Frozen  | [PHASE_55.md](PHASE_55.md) | `8a750a1`  |
| Phase 56 | Frozen  | [PHASE_56.md](PHASE_56.md) | `2128e6a`  |
| Phase 57 | Frozen  | [PHASE_57.md](PHASE_57.md) | `81a6885`  |
| Phase 58 | Frozen  | [PHASE_58.md](PHASE_58.md) | `e512d3b`  |
| Phase 59 | Frozen  | [PHASE_59.md](PHASE_59.md) | `20516f2`  |
| Phase 60 | Frozen  | [PHASE_60.md](PHASE_60.md) | `cff2c4c`  |
| Phase 61 | Frozen  | [PHASE_61.md](PHASE_61.md) | `4e68a06`  |
| Phase 62 | Frozen  | [PHASE_62.md](PHASE_62.md) | `f3ec668`  |
| Phase 63 | Frozen  | [PHASE_63.md](PHASE_63.md) | `055207e`  |
| Phase 64 | Frozen  | [PHASE_64.md](PHASE_64.md) | `5fba6c8`  |
| Phase 65 | Frozen  | [PHASE_65.md](PHASE_65.md) | `661d4bc`  |
| Phase 66 | Frozen  | [PHASE_66.md](PHASE_66.md) | `372f78e`  |
| Phase 67 | Frozen  | [PHASE_67.md](PHASE_67.md) | `fe8cdb1`  |
| Phase 68 | Frozen  | [PHASE_68.md](PHASE_68.md) | `6683f21`  |
| Phase 69 | Frozen  | [PHASE_69.md](PHASE_69.md) | `97b6eff`  |
| Phase 70 | Frozen  | [PHASE_70.md](PHASE_70.md) | `7728a45`  |
| Phase 71 | Frozen  | [PHASE_71.md](PHASE_71.md) | `7d67b06`  |
| Phase 72 | Frozen  | [PHASE_72.md](PHASE_72.md) | `645c901`  |
| Phase 73 | Frozen  | [PHASE_73.md](PHASE_73.md) | `99928ef`  |
| Phase 74 | Frozen  | [PHASE_74.md](PHASE_74.md) | `a2ede10`  |
| Phase 75 | Frozen  | [PHASE_75.md](PHASE_75.md) | `b4aea53`  |
| Phase 76 | Frozen  | [PHASE_76.md](PHASE_76.md) | `aace61a`  |
| Phase 77 | Frozen  | [PHASE_77.md](PHASE_77.md) | `0317cbb`  |
| Phase 78 | Frozen  | [PHASE_78.md](PHASE_78.md) | `02b78f6`  |
| Phase 79 | Frozen  | [PHASE_79.md](PHASE_79.md) | `e763a5c`  |
| Phase 80 | Frozen  | [PHASE_80.md](PHASE_80.md) | `5afbaf5`  |
| Phase 81 | Frozen  | [PHASE_81.md](PHASE_81.md) | `f7a44d0`  |
| Phase 82 | Frozen  | [PHASE_82.md](PHASE_82.md) | `33239ee`  |
| Phase 83 | Frozen  | [PHASE_83.md](PHASE_83.md) | `d915e62`  |
| Phase 84 | Frozen  | [PHASE_84.md](PHASE_84.md) | `ae8b222`  |
| Phase 85 | Frozen  | [PHASE_85.md](PHASE_85.md) | `061b521`  |
| Phase 86 | Frozen  | [PHASE_86.md](PHASE_86.md) | `a331bcb`  |
| Phase 87 | Frozen  | [PHASE_87.md](PHASE_87.md) | `aa51577`  |
| Phase 88 | Frozen  | [PHASE_88.md](PHASE_88.md) | `7a69ac5`  |
| Phase 89 | Frozen  | [PHASE_89.md](PHASE_89.md) | `6e070d5`  |
| Phase 90 | Frozen  | [PHASE_90.md](PHASE_90.md) | `6457f7d`  |
| Phase 91 | Frozen  | [PHASE_91.md](PHASE_91.md) | `cf6a9fd`  |
| Phase 92 | Frozen  | [PHASE_92.md](PHASE_92.md) | `576d9c1`  |
| Phase 93 | Frozen  | [PHASE_93.md](PHASE_93.md) | `f6d5d07`  |
| Phase 94 | Frozen  | [PHASE_94.md](PHASE_94.md) | `d440dea`  |
| Phase 95 | Frozen  | [PHASE_95.md](PHASE_95.md) | `5250efd`  |
| Phase 96 | Frozen  | [PHASE_96.md](PHASE_96.md) | `c56cf30`  |
| Phase 97 | Frozen  | [PHASE_97.md](PHASE_97.md) | `cafbf13`  |
| Phase 98 | Frozen  | [PHASE_98.md](PHASE_98.md) | `59936fc`  |
| Phase 99 | Frozen  | [PHASE_99.md](PHASE_99.md) | `a648bbb`  |
| Phase 100 | Frozen | [PHASE_100.md](PHASE_100.md) | `08e2f28`  |
| Phase 101 | Frozen | [PHASE_101.md](PHASE_101.md) | `4967b11`  |
| Phase 102 | Frozen | [PHASE_102.md](PHASE_102.md) | `13dec8f`  |
| Phase 103 | Frozen | [PHASE_103.md](PHASE_103.md) | `38ce9f4`  |
| Phase 104 | Frozen | [PHASE_104.md](PHASE_104.md) | `701c33a`  |
| Phase 105 | Frozen | [PHASE_105.md](PHASE_105.md) | `a4d719e`  |
| Phase 106 | Frozen | [PHASE_106.md](PHASE_106.md) | `67c879d`  |
| Phase 107 | Frozen | [PHASE_107.md](PHASE_107.md) | `c157a28`  |
| Phase 108 | Frozen | [PHASE_108.md](PHASE_108.md) | `19b029d`  |
| Phase 109 | Frozen | [PHASE_109.md](PHASE_109.md) | `2e8eff3`  |
| Phase 110 | Frozen | [PHASE_110.md](PHASE_110.md) | `d1f19f1`  |
| Phase 111 | Frozen | [PHASE_111.md](PHASE_111.md) | `1bbe8fa`  |
| Phase 112 | Frozen | [PHASE_112.md](PHASE_112.md) | `cb1cb89`  |
| Phase 113 | Frozen | [PHASE_113.md](PHASE_113.md) | `149e0c4`  |
| Phase 114 | Frozen | [PHASE_114.md](PHASE_114.md) | `acef4be`  |
| Phase 115 | Frozen | [PHASE_115.md](PHASE_115.md) | `888d6bb`  |
| Phase 116 | Frozen | [PHASE_116.md](PHASE_116.md) | `4ed30d6`  |
| Phase 117 | Frozen | [PHASE_117.md](PHASE_117.md) | `1a0a4db`  |
| Phase 118 | Frozen | [PHASE_118.md](PHASE_118.md) | `bf259ad`  |
| Phase 119 | Frozen | [PHASE_119.md](PHASE_119.md) | `f6993d3`  |
| Phase 120 | Frozen | [PHASE_120.md](PHASE_120.md) | `25cbc56`  |
| Phase 121 | Frozen | [PHASE_121.md](PHASE_121.md) | `2c558dd`  |
| Phase 122 | Frozen | [PHASE_122.md](PHASE_122.md) | `1897d08`  |
| Phase 123 | Frozen | [PHASE_123.md](PHASE_123.md) | `c8f5cfd`  |
| Phase 124 | Frozen | [PHASE_124.md](PHASE_124.md) | `870efbc`  |
| Phase 125 | Frozen | [PHASE_125.md](PHASE_125.md) | `1f649b1`  |
| Phase 126 | Frozen | [PHASE_126.md](PHASE_126.md) | `59e34bb`  |
| Phase 127 | Frozen | [PHASE_127.md](PHASE_127.md) | `3dd222e`  |
| Phase 128 | Frozen | [PHASE_128.md](PHASE_128.md) | `c6f54b0`  |
| Phase 129 | Frozen | [PHASE_129.md](PHASE_129.md) | `95cdc26`  |
| Phase 130 | Frozen | [PHASE_130.md](PHASE_130.md) | `215734d`  |
| Phase 131 | Frozen | [PHASE_131.md](PHASE_131.md) | `6fb0f38`  |
| Phase 132 | Frozen | [PHASE_132.md](PHASE_132.md) | `904b67c`  |
| Phase 133 | Frozen | [PHASE_133.md](PHASE_133.md) | `0b8811c`  |
| Phase 134 | Frozen | [PHASE_134.md](PHASE_134.md) | `f290caa`  |
| Phase 135 | Frozen | [PHASE_135.md](PHASE_135.md) | `7fdadbd`  |
| Phase 136 | Frozen | [PHASE_136.md](PHASE_136.md) | `0157ce0`  |
| Phase 137 | Frozen | [PHASE_137.md](PHASE_137.md) | `9b6b8a1`  |
| Phase 138 | Frozen | [PHASE_138.md](PHASE_138.md) | `6bf36d6`  |
| Phase 139 | Frozen | [PHASE_139.md](PHASE_139.md) | `38734ec`  |
| Phase 140 | Frozen | [PHASE_140.md](PHASE_140.md) | `45ff66e`  |
| Phase 141 | Frozen | [PHASE_141.md](PHASE_141.md) | `a7e0440`  |
| Phase 142 | Frozen | [PHASE_142.md](PHASE_142.md) | `d46b43c`  |
| Phase 143 | Frozen | [PHASE_143.md](PHASE_143.md) | `182021d`  |
| Phase 144 | Frozen | [PHASE_144.md](PHASE_144.md) | `bb28671`  |
| Phase 145 | Frozen | [PHASE_145.md](PHASE_145.md) | `e0a0b81`  |
| Phase 146 | Frozen | [PHASE_146.md](PHASE_146.md) | `034ff01`  |
| Phase 147 | Frozen | [PHASE_147.md](PHASE_147.md) | `24ced96`  |
| Phase 148 | Frozen | [PHASE_148.md](PHASE_148.md) | `36b6158`  |
| Phase 149 | Frozen | [PHASE_149.md](PHASE_149.md) | `62b7a0c`  |
| Phase 150 | Frozen | [PHASE_150.md](PHASE_150.md) | `5945702`  |
| Phase 151 | Frozen | [PHASE_151.md](PHASE_151.md) | `10a711f`  |
| Phase 152 | Frozen | [PHASE_152.md](PHASE_152.md) | `1002330`  |
| Phase 153 | Frozen | [PHASE_153.md](PHASE_153.md) | `4410b17`  |
| Phase 154 | Frozen | [PHASE_154.md](PHASE_154.md) | `ae518a4`  |
| Phase 155 | Frozen | [PHASE_155.md](PHASE_155.md) | `ad84b49`  |
| Phase 156 | Frozen | [PHASE_156.md](PHASE_156.md) | `302c142`  |
| Phase 157 | Frozen | [PHASE_157.md](PHASE_157.md) | `16c666a`  |
| Phase 158 | Frozen | [PHASE_158.md](PHASE_158.md) | `dda59c2`  |
| Phase 159 | Frozen | [PHASE_159.md](PHASE_159.md) | `2fc8bca`  |
| Phase 160 | Frozen | [PHASE_160.md](PHASE_160.md) | `9f091ff`  |
| Phase 161 | Frozen | [PHASE_161.md](PHASE_161.md) | `7d8c293`  |
| Phase 162 | Frozen | [PHASE_162.md](PHASE_162.md) | `82bc141`  |
| Phase 163 | Frozen | [PHASE_163.md](PHASE_163.md) | `d7b7c24`  |
| Phase 164 | Frozen | [PHASE_164.md](PHASE_164.md) | `7a0de2d`  |
| Phase 165 | Frozen | [PHASE_165.md](PHASE_165.md) | `da63ace`  |
| Phase 166 | Frozen | [PHASE_166.md](PHASE_166.md) | `108bc1c`  |
| Phase 167 | Frozen | [PHASE_167.md](PHASE_167.md) | `75799bd`  |
| Phase 168 | Frozen | [PHASE_168.md](PHASE_168.md) | `92fd777`  |
| Phase 169 | Frozen | [PHASE_169.md](PHASE_169.md) | `e1c5bf4`  |
| Phase 170 | Frozen | [PHASE_170.md](PHASE_170.md) | `7bc29db`  |
| Phase 171 | Frozen | [PHASE_171.md](PHASE_171.md) | `03fad6f`  |
| Phase 172 | Frozen | [PHASE_172.md](PHASE_172.md) | `b0ca1f0`  |
| Phase 173 | Frozen | [PHASE_173.md](PHASE_173.md) | `36610d9`  |
| Phase 174 | Frozen | [PHASE_174.md](PHASE_174.md) | `01d5777`  |
| Phase 175 | Frozen | [PHASE_175.md](PHASE_175.md) | `5cc78eb`  |
| Phase 176 | Frozen | [PHASE_176.md](PHASE_176.md) | `2857c06`  |
| Phase 177 | Frozen | [PHASE_177.md](PHASE_177.md) | `3996fc1`  |
| Phase 178 | Frozen | [PHASE_178.md](PHASE_178.md) | `b3577e7`  |
| Phase 179 | Frozen | [PHASE_179.md](PHASE_179.md) | `0d369d1`  |
| Phase 180 | Frozen | [PHASE_180.md](PHASE_180.md) | `eec1893`  |
| Phase 181 | Frozen | [PHASE_181.md](PHASE_181.md) | `fc8eafe`  |
| Phase 182 | Frozen | [PHASE_182.md](PHASE_182.md) | `c600ab0`  |
| Phase 183 | Frozen | [PHASE_183.md](PHASE_183.md) | `b9e59d0`  |
| Phase 184 | Frozen | [PHASE_184.md](PHASE_184.md) | `ff0463d`  |
| Phase 185 | Frozen | [PHASE_185.md](PHASE_185.md) | _pending_  |

Frozen means the phase doc is no longer edited except through commits
with a message starting `docs(phase-N):` — a convention, not an
enforcement, but it makes drift visible in `git log`.

**Planned phases** (6 and beyond) live in [`ROADMAP.md`](ROADMAP.md)
as one-paragraph intents rather than as separate PHASE_N.md files.
A phase gets its own doc only when it opens — that way there are no
stale task lists sitting in files for phases we haven't started yet.

## Amendment process

When a Phase N decision needs to override a `DESIGN.md` or
`PRODUCT.md` contract:

1. Create `docs/amendments/<date>-<short-slug>.md` — one file per
   amendment, describing what changed, why, and which `DESIGN.md`
   or `PRODUCT.md` section it supersedes (or adds, if it introduces
   a new commitment).
2. Edit the relevant section of `DESIGN.md` or `PRODUCT.md` to
   reference the amendment inline (blockquote-style pointer to the
   amendment file). For added sections (e.g. a new Product
   Commitment), the pointer appears at the head of the new section.
3. Commit the amendment and the contract edit together with message
   `docs(amendment): <slug>` for standalone amendment commits, or as
   a task inside a phase using `docs(phase-N): task M — Ax <slug>`.

The directory holds **ten amendments** as of Phase 56 (2026-05-12):
A1–A4 (Phase 22 — daemon IPC, mission state machine, capability
taxonomy, workspace layout), A5 (Phase 38 — substrate tool count
7→8), A6 (Phase 40 — parallel tool execution), A7 (Phase 41 —
protocol negotiation), A8 (Phase 56 — PRODUCT.md pitch reframe),
A9 (Phase 56 — P13 Assistant Profile), A10 (Phase 56 — P14
Persona).
