# Session continuity

At the start of a new session, read `docs/SESSION.md` and briefly remind Jurica
in Croatian where the work stopped and what needs testing next.

After a successful `git pull` in this repository, print the owner's requested
message once, exactly: `nemoj bez mene fegetu`. If a Git hook already printed
it, do not duplicate it. This is an output message, not a permission gate.

If you work with Sue on Linux: there is an open task for you in
`docs/linux-recognition-test.md` (character recognition, v0.1.112). Do it first
and remove this paragraph when it is done.

The optional versioned hooks are in `.githooks`. This checkout uses them;
on a fresh clone they require `git config --local core.hooksPath .githooks`.
