# encoding

Text as bytes and bytes as text: UTF-8, hex, base64, and the percent encoding
URLs use.

```rust
import "std/encoding";
```

Bytes are an `int[]` whose elements are 0 to 255, the way
[`fs::read_bytes`](fs.md) answers them.

```rust
import "std/encoding";

fn main() {
    let bytes = encoding::utf8("hi");
    print(encoding::hex(bytes));
    print(encoding::base64(bytes));
    print(encoding::url_encode("a b&c"));
}
```

| Function | Returns |
| --- | --- |
| `utf8(s)` | The UTF-8 bytes of the string `s` |
| `from_utf8(bytes)` | The text the bytes spell, as a `string!string`; `Err("encoding_invalid_utf8")` for bytes that are not UTF-8 or an element that is not a byte |
| `hex(bytes)` | The bytes as lowercase hex, two digits each |
| `from_hex(text)` | The bytes the hex digits spell, either case, as an `int[]!string`; `Err("encoding_invalid_hex")` for an odd length or a character that is no digit |
| `base64(bytes)` | The bytes in standard base64, padded with `=` |
| `from_base64(text)` | The bytes, as an `int[]!string`, from the standard or the URL alphabet, padded or not; `Err("encoding_invalid_base64")` for anything else |
| `url_encode(s)` | `s` with every byte outside letters, digits, `-`, `.`, `_` and `~` written as `%XX`, for a path segment or a query value |
| `url_decode(s)` | `s` with each `%XX` turned back into its byte and `+` into a space, as a `string!string`; `Err("encoding_invalid_url")` for a broken escape or bytes that are not UTF-8 |
