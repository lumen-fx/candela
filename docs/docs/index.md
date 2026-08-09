---
icon: lucide/rocket
---

# Get started

## What is Candela, and why?

!!! warning

    Candela is experimental and under active development (this documentation too). Little is set in stone.

Candela is a fast, statically-typed interpreted language that combines Rust-like syntax with Python's ease of use.

It aims to be a much faster alternative to Python that sits closer to low-level languages while staying approachable. You should like Candela whether you are a seasoned Rust developer or you have barely touched Python and are new to programming.

Candela's main selling points are:

- About 10x faster than Python and competitive with LuaJIT (-joff) in our benchmarks
- Statically typed, with full type inference and zero annotations
- FFI support: call C and dynamic libraries directly from Candela with a native, easy syntax
- Embeddable in other programs through a C ABI and a Rust host API

The goal of this documentation / tutorial is to show Candela's syntax and how it works by example more than by theory.

## Installation

### On macOS / Linux

Candela provides a macOS / Linux installer, which you can use to download and install Candela by running the following command in your terminal:<br/>
`#!bash curl -fsSL https://raw.githubusercontent.com/lumen-fx/candela/main/install.sh | sh`

This will install the latest Candela version in `Library/Candela` on macOS, and in `/usr/local/lib/candela/` on Linux.

To install a specific release instead of the latest one, pass `--version`:<br/>
`#!bash curl -fsSL https://raw.githubusercontent.com/lumen-fx/candela/main/install.sh | sh -s -- --version 0.0.1`

Write the release tag with or without its leading `v`. This also pins the install: Candela stays on that release, and the update check below stays quiet, until you install again without `--version`.

### On Windows

Candela doesn't provide a Windows installer yet. You must manually download it from [the latest release on GitHub](https://github.com/lumen-fx/candela/releases/latest).

## Usage

Once installed, you can use the `candela` command like any other:

- To run the REPL, run: `#!bash candela`
- To run a `.cdl` file [^extension], run: `#!bash candela file.cdl`
- To display Candela's current version, run: `#!bash candela -v` or `#!bash candela --version`
- To display the available commands, run: `#!bash candela -h` or `#!bash candela --help`

[^extension]: Candela files have the `.cdl` file extension.
*[REPL]: Read-Eval-Print-Loop

## Staying up to date

When you start the REPL or ask for `candela --help`, Candela asks GitHub for the newest release, at most once a day. If a newer one is out, you get a single line on stderr naming it and the command to install it. Nothing is downloaded or replaced for you.

Running a program never triggers the check, so a script's output and exit status stay its own.

The check is skipped when:

- You pinned the install with `install.sh --version`
- You built Candela from source instead of installing it
- `CANDELA_NO_UPDATE_CHECK` is set to any non-empty value
- `CI` is set, or stderr is not a terminal

To update, run the install command again.

## Benchmarks
![Candela benchmarks](images/candela-benchmarks.png){ loading=lazy }
