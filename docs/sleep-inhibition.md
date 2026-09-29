# Sleep inhibition

Implemented and locally verified on Fedora KDE on 2026-09-19. A reported
long-video interruption prompted the change; standby was a possible cause,
not a confirmed diagnosis or a proven fix.

## Decision and behavior

Use `systemd-inhibit --what=sleep --mode=block --no-ask-password` as a small Linux
adapter: it works with the tested KDE setup without Qt/desktop bindings or new
Rust dependencies. Broader platform integration remains deferred.

CLI casting holds the lock through preflight, discovery, preparation, playback
(including pauses) and cleanup. The KDE helper acquires it when a cast or
conversion starts; opening a window, inspecting, previewing and discovering
alone do not acquire it. CLI/helper `--no-inhibit-sleep` opts out. Missing helper,
denied permission or a three-second acquisition timeout produces a warning and
allows work to continue. Forced sleep/policy overrides may still interrupt it.

[KDE PowerDevil](https://github.com/KDE/powerdevil/blob/master/daemon/powerdevilpolicyagent.cpp)
honors logind sleep inhibitors. The lock does not inhibit `idle`, so dimming and
screen locking remain possible. Other desktop/platform behavior is unverified.

A fixed script emits readiness only after acquiring the lock. A private stdin
pipe keeps it alive; closing the pipe releases it, including after abrupt parent
exit. No media path or device name is passed to the helper. Terminal signals are
isolated so inhibition survives cleanup. Normal release waits up to two seconds,
then kills/reaps a stuck helper. See [systemd's implementation](https://github.com/systemd/systemd/blob/main/src/login/inhibit.c).

## Verification

Fake-helper/CLI tests cover acquisition failure, timeout/cancellation, explicit
and dropped-guard release, reaping, opt-out and SIGINT/SIGTERM cleanup. They do
not alter host power policy. A separate local test used a blocking fake probe,
contacted no receiver, and observed the Yeet lock in logind and KDE
`ActiveInhibitions`; SIGINT returned 130 and removed both entries.

No actual suspend or long-video test was performed for this feature. During a
cast, check the KDE power applet or `systemd-inhibit --list --no-pager`; the Yeet
entry should disappear after cleanup. See [remaining validation](plan.md#open-functional-and-release-work).
