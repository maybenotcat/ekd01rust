# Firmware translations

The UI uses Slint's native gettext support (`@tr`) and bundled PO catalogs.
English is the source language in `ui/*.slint`; it needs no separate PO file.
`ekd01rust.pot` lists the source messages. Russian and Spanish are in
`ru/LC_MESSAGES/ekd01rust.po` and `es/LC_MESSAGES/ekd01rust.po`. Catalogs are compiled into flash; no gettext
installation, locale service or filesystem access is needed on the device.

## Add a language

1. Copy `ekd01rust.pot` to `<locale>/LC_MESSAGES/ekd01rust.po` here. Set the
   `Language`, UTF-8 encoding and appropriate `Plural-Forms` headers. Translate
   every message and remove fuzzy flags.
2. Add `ID | locale | native name` to `languages.txt`. Choose an unused ID in
   2..255. Existing IDs must never change; the file's row order controls the
   selector, not storage identity. No Rust or Slint language branch is required.
3. Run the native tests and build the firmware. Verify glyph coverage and text
   layout on the display; long translations do not automatically resize menus.

English (ID 0) and Russian (ID 1) are permanent for compatibility. Unknown IDs
display English but remain intact in the settings record until the user chooses
a supported language. Settings records v1..v6 still load; the next actual save
migrates them to v7. Record size and filesystem layout are unchanged. V6 introduced
the full locale ID in flags bits 16..23, keeping the old Russian bit separate
from motor mode and theme. V7 adds display rotation in bit 13; older records
default to the new 180-degree orientation. Older firmware cannot read a v7 record.

## Change UI messages

Write English strings with `@tr` in Slint. Use placeholders for dynamic sentences
so translators can reorder arguments; keep numeric formatting in `ui/text.slint`.
Do not put translated strings in Rust or add boolean language switches.
The About labels, missing Git revision and modified marker also use this catalog.
The `English` message is the native name of each language: e.g. `Русский` or
`Español`. It must match the registry's native name. Keeping it in the catalog
also ensures its glyphs enter the embedded font atlas.
Technical identifiers such as CAN, UART, JEDEC and numeric scale ticks stay literal.

Update the template with the Slint 1.15.1 `slint-tr-extractor`, using
`--no-default-translation-context`, then merge it into the PO files with gettext
`msgmerge` (or a PO editor). The firmware build does not require these maintainer
tools. The build sets `DefaultTranslationContext::None`, so entries have no msgctxt.
Preserve whitespace, line breaks and format placeholders in translations.

From the parent directory:

```text
cargo test --locked --manifest-path ekd01rust/tests/parity/Cargo.toml --target host-tuple
```

Tests check catalog completeness against all UI `@tr` calls, the POT, duplicate
keys and fuzzy/empty translations; validate registry IDs and settings migration;
and switch an existing Slint component between English, Russian and Spanish on the PC.
The last check uses the same UI formatter functions and the exact firmware PO
content. It does not emulate the MCU or claim hardware validation.

At runtime `UserInterface::set_language` selects the bundled catalog once per
language change. Slint invalidates translation bindings, including on an already
open page. The selector cycles through the generated registry in both directions.
