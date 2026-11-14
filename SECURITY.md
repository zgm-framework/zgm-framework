# Reporting a vulnerability

Please report vulnerabilities privately, through **Security → Report a vulnerability** in this repository on GitHub (GitHub private vulnerability reporting). Please do not open a public issue until a fix is released.

A useful report has: the module and its version or commit, what the vulnerability allows, steps to reproduce it and, if you have one, a suggested fix.

Fixes go into the latest released version.

## What counts as a vulnerability

Anything that breaks what a module promises. For `zgm-limits`:

- getting around a limit: more attempts or elements than the rule allows;
- reading or changing the counters, rules or blocks of another tenant;
- a hidden part of a key (such as an e-mail address) leaking into the database, the events or the errors;
- a change made by `reset`, `unblock`, `override_rule` or `restore_default` without its event reaching `Audit`.

The section "What to watch out for" in the README of `zgm-limits` describes behavior that is known and intended, such as a block that works against the owner of the account too.
