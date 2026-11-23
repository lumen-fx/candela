# term

The terminal a program writes to: whether there is one, its size, and ANSI
colours and styles for text.

```rust
import "std/term";
```

```rust
import "std/term";

fn main() {
    print(term::color("ok", "green") + " " + term::bold("done"));
    if !term::is_tty() {
        eprint("output is piped");
    }
}
```

The colour and style functions add their codes only when standard output is a
terminal and the `NO_COLOR` variable is not set, so output piped to a file or
another program stays plain text.

| Function | Returns |
| --- | --- |
| `is_tty(stream = "stdout")` | True when `"stdin"`, `"stdout"` or `"stderr"` is a terminal rather than a file or a pipe; always false in a browser |
| `size()` | The terminal's width and height in characters, an `(int, int)`, from the `COLUMNS` and `LINES` variables the shell sets, or `(80, 24)` when they are not set |
| `color(text, name)` | `text` in the colour `name`: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, or one of those after `bright_`; another name leaves the text as it is |
| `bold(text)`, `dim(text)`, `underline(text)` | `text` in that style |
| `strip(text)` | `text` without the ANSI escape sequences in it |
