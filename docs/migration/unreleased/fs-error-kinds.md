# fs raises fs_directory_not_empty and fs_other for failures it had no kind for

A file system failure whose kind the `fs_` error list did not name reached code
the runtime assumed could not run, so what followed was undefined: the program
could stop, or carry on in a broken state, and no `catch` could take it. The most common one was
`fs::delete_dir` on a directory that still has entries. Those failures now
raise catchable errors: `fs_directory_not_empty` for that case, and `fs_other`
for any kind the list still does not name.

This affects you if a program catches the `fs_` errors by kind and relied on
one of these failures never being raised. Catch the new kinds where you catch
the others.

Before, this was undefined:

```rust
import "std/fs";

fn main() {
    fs::mkdir("full");
    fs::write("full/a.txt", "a");
    fs::delete_dir("full");
}
```

After, it raises an error you can catch:

```rust
import "std/fs";

fn main() {
    fs::mkdir("full");
    fs::write("full/a.txt", "a");
    try {
        fs::delete_dir("full");
    } catch "fs_directory_not_empty" {
        print("delete the files first");
    }
}
```
