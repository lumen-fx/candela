# Philosophy

What candela values, in the order it trades them off. When a change forces a
choice, this file says which side wins.

## Fast code

Compiled candela runs fast, and staying fast outranks adding features. The
compiler does the work up front: programs type-check and compile to bytecode
before they run, so the VM executes without per-operation type tests or
lookup overhead. A language feature is judged by what it costs at run time;
one that taxes programs that never use it does not land.

## Small footprint

Programs use little memory, and so does the runtime that hosts them. The
shipping VM stays small enough to embed anywhere; the budget is under 1 MiB.
Values carry no hidden baggage, and the standard library is written in
candela on a small set of native primitives, so nothing heavy hides under a
builtin.

## Beginner friendly

The API reads the way a newcomer guesses it does. Annotations are optional
where the compiler can infer, names say what they mean, errors point at the
mistake and appear before the program runs instead of halfway through it.
There is one way to import, one way to attach methods, one obvious spelling
for the common thing. A surface that needs a tour before first use is
designed wrong.

## Simple, without a ceiling

Easy for newbies to pick up, infinitely flexible for power users. Hello
world is a function and a print. From there the same small core scales up
through structs, enums, generics, closures, and methods without switching
languages or dialects; power comes from composing the pieces already
learned, not from a second, advanced language hiding behind the first.
