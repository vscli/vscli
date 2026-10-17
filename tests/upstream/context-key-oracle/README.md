# Pinned context expression oracle

The JavaScript parser, lexer and character codes are transpiled unchanged from
the Microsoft VS Code commit recorded in provenance.json. Native production does
not load this code. TypeScript is unnecessary when running the checked-in oracle.
The small infrastructure shims supply diagnostic strings/errors, host platform
booleans and an unused dependency-injection decorator; parsing/evaluation stays
in the exact upstream implementation. Browser constants are false.

The runner uses strict regex lexing without error recovery and JSON-owned context
values. Differential evidence qualifies only its listed supported expression/value
cases; it does not establish every JavaScript RegExp feature, platform/context
scope or complete keybinding-resolution parity. Original source and generated
module SHA-256 values and the MIT notice are retained beside the code.
