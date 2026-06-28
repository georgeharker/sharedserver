#!/usr/bin/env bash
# Test server that IGNORES SIGTERM but dies on SIGKILL, staying a single
# long-lived process. Used to exercise `admin stop`'s escalation path:
# plain stop must time out (SIGTERM ignored), and `stop --force` must succeed
# (SIGKILL is not catchable).
#
# The sleep runs in the background and is `wait`ed on so that when SIGTERM is
# delivered to the process group, the sleep child dies and `wait` returns, but
# this shell (which ignores TERM) simply loops and keeps running.
trap '' TERM
echo "ignore_sigterm: running (pid $$)"
while :; do
	sleep 3600 &
	wait $!
done
