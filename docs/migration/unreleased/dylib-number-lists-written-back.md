# A list of ints or floats passed to a C function sees what C writes into it

A list passed to a `dylib` function used to be a copy C could write into
without the program ever seeing it. A list of ints or of floats is now copied
back once the call returns, so whatever the function wrote into the buffer is
in the list afterwards. Lists of strings, lists of lists and structs are still
copies.

This affects you if a program passes a list of numbers to a C function that
writes into the buffer it is handed, and expects the list to be unchanged
afterwards. Pass a copy instead.

Before, `xs` was unchanged after the call:

```rust
dylib "mylib" {
    sort_in_place(int[], int);
}

fn main() {
    let xs = [3, 1, 2];
    mylib::sort_in_place(xs, xs.len());
    print(xs);
}
```

After, it holds what the function wrote: `[1,2,3]`. To keep the original, pass
a copy:

```rust
dylib "mylib" {
    sort_in_place(int[], int);
}

fn main() {
    let xs = [3, 1, 2];
    let scratch = xs[0..];
    mylib::sort_in_place(scratch, scratch.len());
    print(xs);
}
```
