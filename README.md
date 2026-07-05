# latexlings 🦠📄

Small exercises to learn writing, debugging, and typesetting **LaTeX** in the
terminal — inspired by [rustlings](https://github.com/rust-lang/rustlings).

Most exercises are broken `.tex` files: you read the compiler error, fix the
file, and watch it go green. The later ones are full **typesetting
assignments** — you write a document from a spec, and latexlings checks the
*rendered PDF text*, not your source.

```
┌ latexlings ──────────────────────────────── 12/50 done ┐
│ ███████████░░░░░░░░░░░░░░░░░░░░░░░░░░  24%             │
└─────────────────────────────────────────────────────────┘
┌ exercises/06_math_basics/math1.tex [fix] ───────────────┐
│                                                          │
│  ✗ pdflatex failed:                                      │
│                                                          │
│    ./exercises/06_math_basics/math1.tex:18:              │
│    ! Missing $ inserted.                                 │
│                                                          │
└──────────────────────────────────────────────────────────┘
  n:next  h:hint  l:list  c:check-all  r:recompile  ↑↓:scroll  q:quit
```

## Prerequisites

- A TeX distribution (`pdflatex` on your PATH)
  - macOS: `brew install --cask mactex-no-gui`
  - Debian/Ubuntu: `sudo apt install texlive-latex-extra`
- `pdftotext` for the writing assignments
  - macOS: `brew install poppler`
  - Debian/Ubuntu: `sudo apt install poppler-utils`

## Install & start

```sh
cargo install latexlings
latexlings init      # extracts exercises into ./latexlings
cd latexlings
latexlings           # the watch TUI
```

Watch mode auto-opens the current exercise for you (VS Code, a reused
tmux/Zellij pane, or `$EDITOR`) — every `:w` triggers a recompile. Make it
pass, delete the `% I AM NOT DONE` line, press `n`. Want a different editor?
Pass `--edit-cmd <cmd>`. Don't want auto-open at all? Pass `--no-editor`
(or set `LATEXLINGS_NO_EDITOR`).

## Commands

| command | what it does |
|---|---|
| `latexlings` | watch mode: recompiles the current exercise on save |
| `latexlings run <name>` | verify one exercise |
| `latexlings verify` | verify all exercises |
| `latexlings hint <name>` | print the hint |
| `latexlings list` | plain progress list |
| `latexlings reset <name>` | restore an exercise to its shipped state |
| `latexlings solution <name>` | print the reference solution |

After a successful compile, latexlings also runs `chktex` (if installed)
and prints any lint notes it finds. This is best-effort and informational —
a handful of exercises set `strict_chktex` and treat lint warnings as a
hard failure, but most don't.

## The course

`00_intro` marker mechanics · `01_documents` preamble vs body ·
`02_text` reserved characters, quotes, dashes · `03_emphasis` scoped styling ·
`04_lists` itemize/enumerate/description · `05_sections_refs` structure,
labels, ToC · `06_math_basics` inline/display math · `07_math_display`
amsmath environments · `08_math_advanced` delimiters & operators ·
`09_tables` tabular & booktabs · `10_figures` graphicx & floats ·
`11_macros` \newcommand · `12_errors` log-reading drills ·
`13_packages_layout` geometry & layout · `14_bibliography` citations ·
`15_typesetting` full document assignments · `quizzes` the gauntlet.

A companion **field guide** (chapter per topic) lives in the repo that this
course was built alongside; any LaTeX intro works too — the
[Overleaf 30-minute tutorial](https://www.overleaf.com/learn/latex/Learn_LaTeX_in_30_minutes)
covers everything the early exercises need.

## For exercise authors

`latexlings dev-check` (run in the repo) asserts every exercise fails the
way it should as shipped, and every solution compiles and passes its checks.

`latexlings dev-new <category>/<name> [--mode fix|write]` scaffolds a new
exercise + solution skeleton and prints the `info.toml` entry to paste in.

## License

MIT
