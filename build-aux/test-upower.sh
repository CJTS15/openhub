#!/bin/sh
# Runs the UPower tests against python-dbusmock's UPower template, on a
# private bus that stands in for the system bus.
set -eu
exec dbus-run-session -- sh -ec '
  export DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
  python3 -m dbusmock --system --template upower &
  mock=$!
  gdbus wait --system --timeout 10 org.freedesktop.UPower
  status=0
  cargo test -- --ignored upower_mock || status=$?
  kill "$mock"
  exit "$status"'
