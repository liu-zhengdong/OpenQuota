#!/usr/bin/env bash
set -euo pipefail

binary="$(realpath "${1:?Linux release binary is required}")"
tray_host="${2:-unavailable}"
release_validation="${3:-false}"
case "${tray_host}" in
  available | unavailable) ;;
  *)
    echo "Linux tray host must be available or unavailable: ${tray_host}" >&2
    exit 1
    ;;
esac
case "${release_validation}" in
  true | false) ;;
  *)
    echo "Linux release validation must be true or false: ${release_validation}" >&2
    exit 1
    ;;
esac
test -x "${binary}"
export OPENQUOTA_SMOKE_BINARY="${binary}"
export OPENQUOTA_SMOKE_RELEASE_VALIDATION="${release_validation}"
export OPENQUOTA_SMOKE_TRAY_HOST="${tray_host}"

xvfb-run -a dbus-run-session -- bash -euo pipefail -c '
  runner_temp="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
  export HOME
  HOME="$(mktemp -d "${runner_temp}/openquota-x11-home.XXXXXX")"
  export XDG_CONFIG_HOME="${HOME}/xdg"
  export XDG_STATE_HOME="${HOME}/state"
  export XDG_CURRENT_DESKTOP="ubuntu:GNOME"
  export XDG_SESSION_TYPE="x11"
  mkdir -p "${XDG_CONFIG_HOME}" "${XDG_STATE_HOME}"
  stdio_log="${runner_temp}/openquota-x11-app-${RANDOM}.log"
  runtime_log="${XDG_STATE_HOME}/openquota/logs/OpenQuota.log"
  wm_log="${runner_temp}/openquota-x11-openbox-${RANDOM}.log"
  watcher_log="${runner_temp}/openquota-x11-watcher-${RANDOM}.log"
  watcher_pid=""
  wm_pid=""
  app_pid=""
  app_exit_detail=""
  app_exit_status=""
  pending=""

  cleanup() {
    if test -n "${app_pid}"; then
      kill "${app_pid}" 2>/dev/null || true
      wait "${app_pid}" 2>/dev/null || true
    fi
    if test -n "${wm_pid}"; then
      kill "${wm_pid}" 2>/dev/null || true
      wait "${wm_pid}" 2>/dev/null || true
    fi
    if test -n "${watcher_pid}"; then
      kill "${watcher_pid}" 2>/dev/null || true
      wait "${watcher_pid}" 2>/dev/null || true
    fi
  }
  trap cleanup EXIT

  dump_logs() {
    local log
    for log in "${stdio_log}" "${runtime_log}" "${wm_log}" "${watcher_log}"; do
      if test -s "${log}"; then
        echo "----- ${log} -----" >&2
        cat "${log}" >&2 || true
      fi
    done
    if command -v sudo >/dev/null && sudo -n true 2>/dev/null; then
      echo "----- kernel log (recent crash reports) -----" >&2
      sudo -n dmesg 2>/dev/null | grep -E "segfault|traps:|general protection|Out of memory" | tail -n 20 >&2 || true
    fi
  }

  fail() {
    dump_logs
    echo "Linux X11 smoke (${OPENQUOTA_SMOKE_TRAY_HOST} tray host) failed: $*" >&2
    exit 1
  }

  # Reaps an OpenQuota process that is no longer running and records how it
  # ended, so an early exit is reported with its status instead of silently.
  reap_app() {
    local status=0
    wait "${app_pid}" 2>/dev/null || status=$?
    app_pid=""
    app_exit_status="${status}"
    if test "${status}" -gt 128; then
      app_exit_detail="was terminated by signal $(kill -l "$((status - 128))" 2>/dev/null || echo "?") (exit status ${status})"
    else
      app_exit_detail="exited with status ${status}"
    fi
    if test "${status}" -ne 0; then
      local crash
      crash="$(crash_summary)"
      if test -n "${crash}"; then
        app_exit_detail="${app_exit_detail} (crash output: ${crash})"
      fi
    fi
  }

  # Native crashes (Xlib aborts, GDK X errors, Rust panics) only reach the
  # stdio log, and the AppImage runtime can mask the signal behind its own exit
  # status, so the failure reason quotes the first crash lines directly.
  crash_summary() {
    test -f "${stdio_log}" || return 0
    grep -E "^\[xcb\]|Assertion .* failed|Gdk-ERROR|Gdk-CRITICAL|X Window System error|The error was|panicked at" \
      "${stdio_log}" 2>/dev/null | head -n 3 | tr "\n" " " | sed "s/ *$//" || true
  }

  app_running() {
    test -n "${app_pid}" && kill -0 "${app_pid}" 2>/dev/null
  }

  window_visible() {
    xdotool search --onlyvisible --limit 1 --pid "${app_pid}" --name "^OpenQuota$" >/dev/null 2>&1
  }

  runtime_log_has() {
    test -f "${runtime_log}" && grep -Fq "$1" "${runtime_log}"
  }

  # Polls a readiness check until it passes or the deadline expires. The check
  # sets "pending" to the condition it is still waiting for, which becomes the
  # failure reason on timeout. The app exiting early fails immediately.
  wait_for() {
    local timeout_seconds="$1" stage="$2" check="$3"
    local deadline=$((SECONDS + timeout_seconds))
    while true; do
      pending=""
      if ! app_running; then
        reap_app
        fail "OpenQuota ${app_exit_detail} while waiting for ${stage}."
      fi
      if "${check}"; then
        return 0
      fi
      if test "${SECONDS}" -ge "${deadline}"; then
        fail "${stage} did not happen within ${timeout_seconds}s; still waiting for: ${pending}."
      fi
      sleep 0.25
    done
  }

  startup_with_tray() {
    runtime_log_has "desktop integration detected (tray=true)" \
      || { pending="\"desktop integration detected (tray=true)\" in the runtime log"; return 1; }
    runtime_log_has "system tray integration ready" \
      || { pending="\"system tray integration ready\" in the runtime log"; return 1; }
    runtime_log_has "OpenQuota startup completed" \
      || { pending="\"OpenQuota startup completed\" in the runtime log"; return 1; }
  }

  startup_without_tray() {
    runtime_log_has "desktop integration detected (tray=false)" \
      || { pending="\"desktop integration detected (tray=false)\" in the runtime log"; return 1; }
    runtime_log_has "OpenQuota startup completed" \
      || { pending="\"OpenQuota startup completed\" in the runtime log"; return 1; }
    window_visible \
      || { pending="a visible X11 window titled OpenQuota owned by pid ${app_pid}"; return 1; }
  }

  standalone_fallback() {
    runtime_log_has "system tray became unavailable; using standalone window" \
      || { pending="\"system tray became unavailable; using standalone window\" in the runtime log"; return 1; }
    window_visible \
      || { pending="a visible X11 window titled OpenQuota owned by pid ${app_pid}"; return 1; }
  }

  watcher_owns_name() {
    if ! kill -0 "${watcher_pid}" 2>/dev/null; then
      fail "The StatusNotifier watcher (dbus-test-tool) exited before acquiring its D-Bus name."
    fi
    dbus-send --session --dest=org.freedesktop.DBus --type=method_call --print-reply \
      /org/freedesktop/DBus org.freedesktop.DBus.NameHasOwner \
      string:org.kde.StatusNotifierWatcher 2>/dev/null | grep -Fq "boolean true"
  }

  command -v wmctrl >/dev/null || {
    echo "wmctrl is required to close the OpenQuota window through the window manager." >&2
    exit 1
  }

  if test "${OPENQUOTA_SMOKE_TRAY_HOST}" = available; then
    command -v dbus-test-tool >/dev/null || {
      echo "dbus-test-tool is required for the Linux tray-host smoke test." >&2
      exit 1
    }
    unset OPENQUOTA_LINUX_TRAY_HOST
    dbus-test-tool echo --session --name=org.kde.StatusNotifierWatcher \
      >"${watcher_log}" 2>&1 &
    watcher_pid=$!
    watcher_deadline=$((SECONDS + 20))
    until watcher_owns_name; do
      if test "${SECONDS}" -ge "${watcher_deadline}"; then
        fail "The StatusNotifier watcher did not acquire its D-Bus name within 20s."
      fi
      sleep 0.25
    done
  else
    export OPENQUOTA_LINUX_TRAY_HOST="unavailable"
  fi

  openbox >"${wm_log}" 2>&1 &
  wm_pid=$!

  "${OPENQUOTA_SMOKE_BINARY}" >"${stdio_log}" 2>&1 &
  app_pid=$!
  if test "${OPENQUOTA_SMOKE_TRAY_HOST}" = available; then
    wait_for 30 "tray-host startup" startup_with_tray
    if ! kill "${watcher_pid}" 2>/dev/null; then
      fail "The StatusNotifier watcher exited on its own before the smoke test stopped it."
    fi
    wait "${watcher_pid}" 2>/dev/null || true
    watcher_pid=""
    wait_for 30 "the standalone window after the tray host stopped" standalone_fallback
  else
    wait_for 30 "standalone startup without a tray host" startup_without_tray
    if runtime_log_has "system tray integration ready"; then
      fail "OpenQuota created a tray while the tray host was unavailable."
    fi
  fi

  # Close the window the way a user does: wmctrl asks openbox, which sends
  # WM_DELETE_WINDOW. "xdotool windowclose" destroys the X window underneath
  # GTK instead, and WebKit drawing into the destroyed window then aborts with
  # a random BadDrawable/BadWindow error. The window can be briefly unmapped
  # while the window manager settles, so retry locating and closing it until
  # the deadline instead of once. An exit after a close attempt counts as a
  # requested close.
  close_attempted=false
  close_requested=false
  close_deadline=$((SECONDS + 10))
  while test "${SECONDS}" -lt "${close_deadline}"; do
    if ! app_running; then
      if test "${close_attempted}" = true; then
        close_requested=true
        break
      fi
      reap_app
      fail "OpenQuota ${app_exit_detail}; it exited before its standalone window was closed."
    fi
    window_id="$(xdotool search --onlyvisible --limit 1 --pid "${app_pid}" --name "^OpenQuota$" 2>/dev/null || true)"
    if test -n "${window_id}"; then
      close_attempted=true
      if wmctrl -i -c "${window_id}" 2>/dev/null; then
        close_requested=true
        break
      fi
    fi
    sleep 0.25
  done
  if test "${close_requested}" != true; then
    fail "OpenQuota did not keep a visible standalone window available for closing within 10s."
  fi

  exit_deadline=$((SECONDS + 20))
  while app_running; do
    if test "${SECONDS}" -ge "${exit_deadline}"; then
      fail "OpenQuota did not exit when its standalone window was closed (waited 20s)."
    fi
    sleep 0.25
  done
  reap_app
  if test "${app_exit_status}" -ne 0; then
    fail "OpenQuota ${app_exit_detail} after its standalone window was closed; expected status 0."
  fi
'
