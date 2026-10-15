#!/bin/sh
# Runs the UPower tests against python-dbusmock's UPower template, on a
# private bus that stands in for the system bus. The tests share one mock,
# so they run one at a time.
set -eu
exec dbus-run-session -- sh -ec '
  export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
  python3 -m dbusmock --system --template upower &
  mock=$!
  gdbus wait --system --timeout 10 org.freedesktop.UPower
  status=0
  cargo test -- --ignored --test-threads=1 upower_mock || status=$?
  kill "$mock"
  exit "$status"'
