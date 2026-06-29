# Export Semantics

Use these rules when authoring or editing Djot notes meant for `djot-export`.
`djot-export` produces a pandoc JSON AST; a downstream pandoc command converts it
further (`djot-export doc.dj | pandoc -f json -o doc.pdf`). Pandoc's native Djot
reader handles the base syntax; `djot-export` adds the conventions below on top
of the resulting AST.

## Metadata Block

A leading code block carrying the `metadata` class holds TOML document metadata:

````djot
{.metadata}
``` toml
title = "Usage Guide"
author = "wrvsrx"
created = "2026-06-16T12:34:56+08:00"
```
````

- `djot-export` folds this TOML into pandoc metadata instead of rendering the
  block as body content.
- Tools use the **first** code block carrying the `metadata` class.
- Defined semantic fields are intentionally minimal: `title` (human-facing
  document name) and `created` (RFC 3339 datetime). Other keys are passed through
  to pandoc metadata.
- The body must be valid TOML. Invalid TOML is dropped, not rendered.
- Every metadata string value is parsed as djot markup, exactly as pandoc parses
  YAML metadata scalars: any field (`title`, `subtitle`, `author`, `abstract`,
  ...) may carry emphasis, math, or a `[@key]{.cite}` citation. A single
  paragraph becomes `MetaInlines`, longer content `MetaBlocks`; booleans stay
  boolean and containers recurse. Note this means a leading backslash before
  punctuation is an escape (`\|` becomes `|`), the same as in pandoc Markdown, so
  TeX-ish values like a `norm` macro need `\lVert`/`\rVert` rather than `\|`.

## Citations

A `[X]{.cite}` span is rewritten to a pandoc `Cite` node. The span content `X` is
treated exactly as the body of a pandoc-markdown citation bracket `[X]`, so the
`@key` sigil is required (it names the citation key; CSL itself has no `@`).
Supported forms mirror pandoc-markdown:

```djot
[@smith2004]{.cite}              plain citation -> (Smith 2004)
[-@smith2004]{.cite}             suppress author -> (2004)
[@smith2004, pp. 33-35]{.cite}   locator/suffix -> (Smith 2004, 33-35)
[see @smith2004]{.cite}          prefix -> (see Smith 2004)
[@smith2004; @doe2010]{.cite}    multiple keys -> (Smith 2004; Doe 2010)
```

Rules:

- Keep the whole span content a single citation. Multiple references go in one
  span separated by `;`; do not bury a key inside arbitrary prose.
- A `.cite` span whose content is not a valid citation (no `@`) is left unchanged
  and `djot-export` emits a stderr warning.
- `djot-export` only produces the `Cite` nodes. Resolve them downstream with
  citeproc and a CSL-JSON bibliography:

```sh
djot-export doc.dj | pandoc -f json -o doc.pdf --citeproc --bibliography refs.json
```
