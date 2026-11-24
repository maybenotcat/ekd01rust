# UI structure

`app.slint` selects the screen and binds application data. It contains no screen
geometry or menu pagination.

- `standard.slint` and `classic.slint`: the two main screens.
- `settings.slint`: setting rows, selection and five-row pagination.
- `menu-labels.slint`: English/Russian menu labels and page titles.
- `text.slint`: all other translations, numeric formatting, units and messages.
- `ride.slint`: ride statistics cards.
- `information.slint`: information and diagnostics text.
- `components.slint`: shared page frame and summary card.
- `types.slint`: typed screen, page, item and data contracts.

Open an individual screen in the Slint preview to see sample data. For settings,
change `page`, `items` and `selected` to preview another page or selection.
Application bindings replace sample values before rendering on the device.

Rust owns navigation, item order, editing rules, data validity, unit conversion
and hardware operations. It sends typed states and numeric data, never translated
labels or preformatted user messages. `src/ui/bindings.rs` converts domain enums to Slint enums with
exhaustive matches. The presenter sends all items for the current menu page;
Slint chooses the visible five and looks up their labels. Do not duplicate the
navigation tree in Slint or reintroduce user-facing text in Rust. UART diagnostics
are independent of UI localization.

`DisplayValue` carries a numeric value, fractional digits and validity. Slint
adds units, padding and localized choices. Large counters use base-10000 parts
so all u32 digits survive the embedded renderer's floating-point arithmetic.
`StatusCode` covers operation and power-off messages. Missing data remains
explicit and must not be rendered as a valid zero.
