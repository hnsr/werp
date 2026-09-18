# Sleep inhibition

Implemented and verified on Fedora KDE on 2026-09-19. A reported long-video
interruption prompted this change; standby was a possible explanation, not a
confirmed diagnosis. Procast previously had no sleep inhibition.

## Initial implementation

The backend casting session starts `systemd-inhibit` with `--what=sleep`,
`--mode=block`, and `--no-ask-password`. The lock is labelled Procast with a
generic preparation/casting reason. No media paths or device names are supplied.
This needs no new Rust dependencies or Qt/KDE bindings. The existing Fedora KDE
installation already provides the helper.

[KDE PowerDevil](https://github.com/KDE/powerdevil/blob/master/daemon/powerdevilpolicyagent.cpp)
tracks logind sleep inhibitors as interruptions of the session to prevent.
The lock uses only `sleep`, not `idle`: screen dimming and screen locking can
continue. Other desktop environments/platforms have not been validated for this
feature. Broader integration remains deferred until frontend work.

The inhibitor covers inspection, discovery, media preparation, playback, pauses,
and shutdown cleanup. `inspect` and `devices` do not acquire it. Use
`--no-inhibit-sleep` to opt out of the default for one cast. If the helper is
missing, permission is denied, or acquisition exceeds three seconds, Procast
warns and continues casting. Unexpected helper exit also produces a warning.
Forced sleep and desktop policy overrides can still interrupt playback.

The helper runs a fixed shell script with no interpolated user data. Its readiness
byte is emitted only after systemd acquires the lock. A private stdin pipe holds
the helper open; closing it releases the lock, including when Procast exits
abruptly. The helper is isolated from terminal signals so the lock can survive
through Procast's cleanup. Normal shutdown closes the pipe, waits up to two
seconds, then kills/reaps a helper that does not exit. Dropping the guard requests
the same cleanup. See [systemd's implementation](https://github.com/systemd/systemd/blob/main/src/login/inhibit.c).

## Verification

- All 55 ordinary workspace tests passed, together with formatting and Clippy.
  The six existing opt-in FFmpeg tests were not rerun for this change.
- Fake-helper tests verify readiness failure, missing executable, acquisition
  timeout/cancellation, explicit release, dropped-guard release, and process reaping.
- CLI tests verify best-effort failure, opt-out, release after failed input
  validation, and cleanup with exit 130/143 after SIGINT/SIGTERM. Inspection does
  not create an inhibitor. Ordinary tests do not alter host power policy.
- A separate live local check started Procast with a blocking fake probe and no
  receiver contact. The Procast `sleep`/`block` lock appeared in logind and in
  KDE's `ActiveInhibitions`. SIGINT returned 130 and removed both entries.

No actual suspend or long-video test was performed. Existing deferred seeking
and long-duration playback checks remain deferred.

During an ordinary cast, the CLI should report
`Sleep inhibition active for this casting session`. The lock is also visible in
KDE's power applet or with:

```sh
systemd-inhibit --list --no-pager
```

The Procast entry should disappear after playback ends or Ctrl+C cleanup finishes.
