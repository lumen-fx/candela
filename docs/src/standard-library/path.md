# path

Paths as strings: joining them and taking them apart. Nothing here touches the
disk; [fs](fs.md) does that.

```rust
import "std/path";
```

```rust
import "std/path";

fn main() {
    let p = path::join(["assets", "img", "logo.png"]);
    print(p);
    print(path::parent(p));
    print(path::stem(p) + " is a " + path::extension(p));
}
```

A separator is `/`. On Windows `\` is one too, and results are written with
`\`.

## join

```rust
path::join(parts)
```

- `parts`: a `string[]`.
- Returns: the parts joined with the separator. An empty part is skipped, and
  an absolute part starts the path over from it, so `join(["a", "/b"])` is
  `/b`.

## parent

```rust
path::parent(p)
```

- Returns: everything before the last segment: `a/b` for `a/b/c`, `/` for
  `/a`, and an empty string for a path of one segment.

## name

```rust
path::name(p)
```

- Returns: the last segment: `c.txt` for `a/b/c.txt`.

## stem

```rust
path::stem(p)
```

- Returns: the last segment without its extension: `c` for `a/c.txt`, `c.tar`
  for `c.tar.gz`. A name that starts with a dot, such as `.bashrc`, has no
  extension.

## extension

```rust
path::extension(p)
```

- Returns: the extension of the last segment without the dot, `txt` for
  `a/c.txt`, or an empty string when there is none.

## normalize

```rust
path::normalize(p)
```

- Returns: `p` with `.` segments, repeated separators and `x/..` pairs taken
  out, so `a/./b/../c` is `a/c`. A relative path keeps the `..` it cannot
  resolve, and an empty result is `.`.

## is_absolute

```rust
path::is_absolute(p)
```

- Returns: true when `p` starts at the root: with a separator, or on Windows
  with a drive such as `C:`.

## separator

```rust
path::separator()
```

- Returns: the separator results are written with: `\` on Windows, `/`
  elsewhere.
