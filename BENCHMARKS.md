# Candela benchmarks

These are the programs used to compare Candela against Python 3 and LuaJIT with
the JIT disabled (`-joff`). Each benchmark is the same workload written three
times, once per language, so the three runs are directly comparable.

Candela is still experimental, so treat any figure you measure as a snapshot of
the commit you measured.

## Running them

Benchmark a release build; a debug build is one optimisation level down and
tells you nothing useful.

```sh
cargo build --release
```

Run one workload in each language:

```sh
./target/release/candela examples/fib/fib.cdl
python3 examples/fib/fib.py
luajit -joff examples/fib/fib.lua
```

To get comparable timings, run the three under
[hyperfine](https://github.com/sharkdp/hyperfine):

```sh
hyperfine --runs 150 --warmup 10 \
  './target/release/candela examples/fib/fib.cdl' \
  'python3 examples/fib/fib.py' \
  'luajit -joff examples/fib/fib.lua'
```

Swap the paths for any other benchmark below. Several benchmarks are listed
inline rather than as files; save each program to a file and run it the same
way.

Release binaries are built with profile-guided optimisation, so a locally built
`--release` binary is slower than a published one. The instrumented binary is
trained on the programs in `pgo/`, which include smaller-input versions of the
FizzBuzz and standard-library benchmarks below. The workflow is
[release.yml](.github/workflows/release.yml).

## Iterative fib(46) x 200000

| Candela | Python 3 | LuaJIT (-joff) |
| --- | --- | --- |
| [iter_fib.cdl](/examples/iter_fib/iter_fib.cdl) | [iter_fib.py](/examples/iter_fib/iter_fib.py) | [iter_fib.lua](/examples/iter_fib/iter_fib.lua) |

## Recursive fib(10,15,20,25,30,33)

| Candela | Python 3 | LuaJIT (-joff) |
| --- | --- | --- |
| [fib.cdl](/examples/fib/fib.cdl) | [fib.py](/examples/fib/fib.py) | [fib.lua](/examples/fib/fib.lua) |

## N-body (N=500000)

Based on [this benchmark from The Computer Language Benchmarks Game](https://benchmarksgame-team.pages.debian.net/benchmarksgame/description/nbody.html#nbody).
`nbody_lua` is based on [the fastest Lua implementation](https://benchmarksgame-team.pages.debian.net/benchmarksgame/program/nbody-lua-2.html).
`nbody_py` is based on [the fastest Python implementation](https://benchmarksgame-team.pages.debian.net/benchmarksgame/program/nbody-python3-1.html).

| Candela | Python 3 | LuaJIT (-joff) |
| --- | --- | --- |
| [nbody_lua.cdl](/examples/nbody/nbody_lua.cdl) | [nbody_lua.py](/examples/nbody/nbody_lua.py) | [nbody_lua.lua](/examples/nbody/nbody_lua.lua) |
| [nbody_py.cdl](/examples/nbody/nbody_py.cdl) | [nbody_py.py](/examples/nbody/nbody_py.py) | [nbody_py.lua](/examples/nbody/nbody_py.lua) |

## Binary trees (N=16)

Based on [this benchmark from The Computer Language Benchmarks Game](https://benchmarksgame-team.pages.debian.net/benchmarksgame/description/binarytrees.html#binarytrees), [this Python implementation](https://benchmarksgame-team.pages.debian.net/benchmarksgame/program/binarytrees-python3-2.html) and [this Lua implementation](https://benchmarksgame-team.pages.debian.net/benchmarksgame/program/binarytrees-lua-2.html).

| Candela | Python 3 | LuaJIT (-joff) |
| --- | --- | --- |
| [binary-trees.cdl](/examples/binary-trees/binary-trees.cdl) | [binary-trees.py](/examples/binary-trees/binary-trees.py) | [binary-trees.lua](/examples/binary-trees/binary-trees.lua) |

## Quicksort (N=10000)

| Candela | Python 3 | LuaJIT (-joff) |
| --- | --- | --- |
| [quicksort.cdl](/examples/quicksort/quicksort.cdl) | [quicksort.py](/examples/quicksort/quicksort.py) | [quicksort.lua](/examples/quicksort/quicksort.lua) |

## Sqrt (N=0 to 9999999)

Candela:

```rust
fn main() {
    let x = 0.0;
    for i in 0..10000000 {
        x += float(i).sqrt();
    }
    print(x);
}
```

Python 3:

```python
from math import sqrt

x = 0.0
for i in range(10000000):
    x += sqrt(i)
print(x)
```

LuaJIT:

```lua
local x = 0.0
for i = 0, 9999999 do
    x = x + math.sqrt(i)
end
print(x)
```

## String.split(), Array.contains() x 50000

Candela:

```rust
fn main() {
    let s = "the quick brown fox";
    let count = 0;
    for _ in 0..50000 {
        let parts = s.split(" ");
        if parts.contains("fox") {
            count += 1;
        }
    }
    print(count);
}
```

Python 3:

```python
s = "the quick brown fox"
count = 0
for _ in range(50000):
    parts = s.split(" ")
    if "fox" in parts:
        count += 1
print(count)
```

LuaJIT:

```lua
local s = "the quick brown fox"
local count = 0
for _ = 1, 50000 do
    if s:find("fox") then
        count = count + 1
    end
end
print(count)
```

## FizzBuzz, 1000000 iterations

Candela:

```rust
fn main() {
    let last = "";
    for i in 1..1000001 {
        if i % 15 == 0 {
            last = "FizzBuzz";
        } else if i % 3 == 0 {
            last = "Fizz";
        } else if i % 5 == 0 {
            last = "Buzz";
        } else {
            last = str(i);
        }
    }
    print(last);
}
```

Python 3:

```python
last = ""
for i in range(1, 1000001):
    if i % 15 == 0:
        last = "FizzBuzz"
    elif i % 3 == 0:
        last = "Fizz"
    elif i % 5 == 0:
        last = "Buzz"
    else:
        last = str(i)
print(last)
```

LuaJIT:

```lua
local last = ""
for i = 1, 1000000 do
    if i % 15 == 0 then
        last = "FizzBuzz"
    elseif i % 3 == 0 then
        last = "Fizz"
    elseif i % 5 == 0 then
        last = "Buzz"
    else
        last = tostring(i)
    end
end
print(last)
```

## Standard library operations x 100000

This covers most of the standard library. File system functions are left out so
that IO does not interfere with the measurement.

Candela:

```rust
fn main() {
    let count = 0;
    for _ in 0..100000 {
        let s = "  Hello, World!  ";
        let t = s.trim();
        let tl = s.trim_left();
        let tr = s.trim_right();
        let ts = "-Hello-".trim_sequence("-");
        let tsl = "-Hello-".trim_sequence_left("-");
        let tsr = "-Hello-".trim_sequence_right("-");
        let u = t.uppercase();
        let l = u.lowercase();
        let c = t.contains("World");
        let f = t.find("World");
        let sw = t.starts_with("Hello");
        let ew = t.ends_with("!");
        let isf = "3.14".is_float();
        let isi = "42".is_int();
        let parts = l.split(", ");
        let joined = parts.join("-");
        let r = joined.replace("-", " ");
        let length = r.len();
        let rev = r.reverse();
        let rep = "ab".repeat(3);
        let n = 42.7;
        let sq = n.sqrt();
        let fl = n.floor();
        let ro = n.round();
        let ab = (-5).abs();
        let fab = (-3.14).abs();
        let to_f = float(42);
        let to_i = int(3.14);
        let to_s = str(42);
        let to_b = bool("true");
        let rng = range(10);
        let arr = [3, 1, 4, 1, 5];
        arr.sort();
        arr.reverse();
        count += length;
    }
    print(count);
}
```

Python 3:

```python
import math
count = 0
for _ in range(100000):
    s = "  Hello, World!  "
    t = s.strip()
    tl = s.lstrip()
    tr = s.rstrip()
    ts = "-Hello-".strip("-")
    tsl = "-Hello-".lstrip("-")
    tsr = "-Hello-".rstrip("-")
    u = t.upper()
    l = u.lower()
    c = "World" in t
    f = t.find("World")
    sw = t.startswith("Hello")
    ew = t.endswith("!")
    isf = True
    isi = True
    parts = l.split(", ")
    joined = "-".join(parts)
    r = joined.replace("-", " ")
    length = len(r)
    rev = r[::-1]
    rep = "ab" * 3
    n = 42.7
    sq = math.sqrt(n)
    fl = math.floor(n)
    ro = round(n)
    ab = abs(-5)
    fab = abs(-3.14)
    to_f = float(42)
    to_i = int(3.14)
    to_s = str(42)
    to_b = bool("true")
    rng = list(range(10))
    arr = [3, 1, 4, 1, 5]
    arr.sort()
    arr.reverse()
    count += length
print(count)
```

LuaJIT:

```lua
local count = 0
for _ = 1, 100000 do
    local s = "  Hello, World!  "
    local t = s:match("^%s*(.-)%s*$")
    local tl = s:match("^%s*(.*)")
    local tr = s:match("(.-)%s*$")
    local ts = ("-Hello-"):match("^%-(.-)%-$")
    local tsl = ("-Hello-"):match("^%-(.*)")
    local tsr = ("-Hello-"):match("(.-)%-$")
    local u = t:upper()
    local l = u:lower()
    local c = t:find("World") ~= nil
    local f = t:find("World")
    local sw = t:sub(1,5) == "Hello"
    local ew = t:sub(-1) == "!"
    local isf = tonumber("3.14") ~= nil
    local isi = tonumber("42") ~= nil
    local parts = {}
    for p in l:gmatch("[^,]+") do
        parts[#parts+1] = p
    end
    local joined = table.concat(parts, "-")
    local r = joined:gsub("-", " ")
    local length = #r
    local rev = r:reverse()
    local rep = ("ab"):rep(3)
    local n = 42.7
    local sq = math.sqrt(n)
    local fl = math.floor(n)
    local ro = math.floor(n + 0.5)
    local ab = math.abs(-5)
    local fab = math.abs(-3.14)
    local to_f = 42 + 0.0
    local to_i = math.floor(3.14)
    local to_s = tostring(42)
    local to_b = ("true") == "true"
    local rng = {}
    for i = 0, 9 do rng[#rng+1] = i end
    local arr = {3, 1, 4, 1, 5}
    table.sort(arr)
    local j = 1
    local k = #arr
    while j < k do
        arr[j], arr[k] = arr[k], arr[j]
        j = j + 1
        k = k - 1
    end
    count = count + length
end
print(count)
```

## C FFI call overhead x 10000000

Each program calls the same shared C library function in a loop. The C function
is trivial, so what you measure is the cost of crossing the language-to-C
boundary rather than the work on the other side of it.

`bench_ffi.c`, compiled with `-O2`:

```c
int increment(int x) {
    return x + 1;
}
```

Build it as a shared library named `bench_ffi`, with whatever extension your
platform uses. Candela resolves the extension itself when you leave it off the
`dylib` path.

Candela:

```rust
dylib "./bench_ffi" {
    int increment(int);
}

fn main() {
    let x = 0;
    for _ in 0..10000000 {
        x = bench_ffi::increment(x);
    }
    print(x);
}
```

Python 3:

```python
import ctypes

lib = ctypes.CDLL("./bench_ffi.so")
lib.increment.restype = ctypes.c_int
lib.increment.argtypes = [ctypes.c_int]

x = 0
for _ in range(10_000_000):
    x = lib.increment(x)
print(x)
```

LuaJIT:

```lua
local ffi = require("ffi")
ffi.cdef[[
    int increment(int x);
]]
local lib = ffi.load("./bench_ffi")

local x = 0
for _ = 1, 10000000 do
    x = lib.increment(x)
end
print(x)
```
