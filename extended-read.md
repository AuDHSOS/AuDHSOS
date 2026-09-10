
ER-1:
Reason: /opt/local/bin/rustc (MacPorts) is listed before ~/.cargo/bin in the
PATH on this machine, but the workspace requires the pinned nightly build. The
wrappers correct the PATH, disable pagers and ink, and then execute Cargo: the
output and exit status are those of xtask, so && and $? are valid. A full check
writes around three thousand lines; `sh tools/xtask-check.sh --quiet` reduces this
to one line per step and only displays the output of the step that fails.
