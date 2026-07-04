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
  n:next  h:hint  l:list  r:recompile  ↑↓:scroll  q:quit
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

Open the shown `.tex` file in vim in another pane. Every `:w` triggers a
recompile. Make it pass, delete the `% I AM NOT DONE` line, press `n`.

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

## License

MIT
