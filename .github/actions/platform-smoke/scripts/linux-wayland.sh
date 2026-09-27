#!/usr/bin/env bash
set -euo pipefail

binary="$(realpath "${1:?Linux release binary is required}")"
release_validation="${2:-false}"
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

dbus-run-session -- bash -euo pipefail -c '
  runner_temp="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
  export HOME
  HOME="$(mktemp -d "${runner_temp}/openquota-wayland-home.XXXXXX")"
  export XDG_CONFIG_HOME="${HOME}/xdg"
  export XDG_STATE_HOME="${HOME}/state"
  export XDG_CURRENT_DESKTOP="KDE"
  export XDG_RUNTIME_DIR
  XDG_RUNTIME_DIR="$(mktemp -d "${runner_temp}/openquota-wayland-runtime.XXXXXX")"
  export XDG_SESSION_TYPE="wayland"
  export OPENQUOTA_LINUX_TRAY_HOST="unavailable"
  export GDK_BACKEND="wayland"
  export WAYLAND_DISPLAY="openquota-wayland"
  mkdir -p "${XDG_CONFIG_HOME}" "${XDG_STATE_HOME}"
  chmod 700 "${XDG_RUNTIME_DIR}"
  stdio_log="${runner_temp}/openquota-wayland-app-${RANDOM}.log"
  runtime_log="${XDG_STATE_HOME}/openquota/logs/OpenQuota.log"
  weston_log="${runner_temp}/openquota-weston-${RANDOM}.log"
  weston_pid=""
  app_pid=""
  app_exit_detail=""
  cleanup() {
    if test -n "${app_pid}"; then
      kill "${app_pid}" 2>/dev/null || true
      wait "${app_pid}" 2>/dev/null || true
    fi
    if test -n "${weston_pid}"; then
      kill "${weston_pid}" 2>/dev/null || true
      wait "${weston_pid}" 2>/dev/null || true
    fi
  }
  trap cleanup EXIT

  dump_logs() {
    local log
    for log in "${stdio_log}" "${runtime_log}" "${weston_log}"; do
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
    echo "Linux Wayland smoke failed: $*" >&2
    exit 1
  }

  reap_app() {
    local status=0
    wait "${app_pid}" 2>/dev/null || status=$?
    app_pid=""
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

  runtime_log_has() {
    test -f "${runtime_log}" && grep -Fq "$1" "${runtime_log}"
  }

  weston --backend=headless-backend.so --socket="${WAYLAND_DISPLAY}" --idle-time=0 \
    --log="${weston_log}" &
  weston_pid=$!
  weston_deadline=$((SECONDS + 20))
  until test -S "${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY}"; do
    if ! kill -0 "${weston_pid}" 2>/dev/null; then
      fail "weston exited before creating its Wayland socket."
    fi
    if test "${SECONDS}" -ge "${weston_deadline}"; then
      fail "weston did not create its Wayland socket within 20s."
    fi
    sleep 0.25
  done

  "${OPENQUOTA_SMOKE_BINARY}" >"${stdio_log}" 2>&1 &
  app_pid=$!
  ready_deadline=$((SECONDS + 30))
  while true; do
    if ! kill -0 "${app_pid}" 2>/dev/null; then
      reap_app
      fail "OpenQuota ${app_exit_detail} before reporting a ready Wayland fallback."
    fi
    if ! runtime_log_has "desktop integration detected (tray=false)"; then
      pending="\"desktop integration detected (tray=false)\" in the runtime log"
    elif ! runtime_log_has "OpenQuota startup completed"; then
      pending="\"OpenQuota startup completed\" in the runtime log"
    else
      break
    fi
    if test "${SECONDS}" -ge "${ready_deadline}"; then
      fail "OpenQuota did not report a ready Wayland fallback before the startup deadline (30s); still waiting for: ${pending}."
    fi
    sleep 0.25
  done
  if runtime_log_has "system tray integration ready"; then
    fail "OpenQuota created a tray while the Wayland tray host was unavailable."
  fi
'
