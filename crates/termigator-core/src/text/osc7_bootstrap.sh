# Runs on the remote host, in POSIX sh, in place of the login shell's own start (see
# `tracking_remote_command`). It starts the user's shell as a login shell with a prompt hook that
# reports $PWD with OSC 7, so the Files panel can follow `cd`. A shell it does not know, or any
# failure, falls through to the caller's plain `exec "$SHELL" -l`: nothing here may `exit`.
# A command given to ssh makes sshd skip the MOTD and "Last login", which only an interactive login
# prints: show them here, as sshd would (not with ~/.hushlogin, not without a terminal). A local
# start (WSL) sets ERN_NO_MOTD: no login happened there.
if [ -z "${ERN_NO_MOTD:-}" ] && [ -t 1 ] && [ ! -e "${HOME:-/}/.hushlogin" ]; then
  ern_prev=""
  for ern_f in /run/motd.dynamic /etc/motd; do
    if [ -s "$ern_f" ] && { [ -z "$ern_prev" ] || [ ! "$ern_f" -ef "$ern_prev" ]; }; then
      cat "$ern_f"
      ern_prev="$ern_f"
    fi
  done
  # The session being started is already in wtmp: the one before it is the last login.
  if command -v last >/dev/null 2>&1; then
    ern_tty=$(tty 2>/dev/null)
    last -F -w -a -n 2 "$(id -un)" 2>/dev/null | awk -v t="${ern_tty#/dev/}" '
      NF > 6 && $1 != "wtmp" && $1 != "btmp" { n++; if (n == 1 && $2 == t) next
        h = $NF; d = $3 " " $4 " " $5 " " $6 " " $7
        if (h ~ /^\(/ || h == "in") printf "Last login: %s on %s\n", d, $2
        else printf "Last login: %s from %s\n", d, h
        exit }'
  fi
fi

ern_sh="${SHELL:-}"
if [ -n "$ern_sh" ] && [ -x "$ern_sh" ]; then
  case "${ern_sh##*/}" in
    bash)
      PROMPT_COMMAND='printf "\033]7;file://%s%s\007" "${HOSTNAME:-}" "$PWD"'"${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
      export PROMPT_COMMAND
      exec "$ern_sh" -l
      ;;
    zsh)
      # zsh only reads its hooks from files: a throw-away ZDOTDIR forwards to the real ones and
      # adds the hook after .zshrc; .zlogin (read last) puts ZDOTDIR back and removes it.
      ERN_ZTMP=$(mktemp -d "${TMPDIR:-/tmp}/termigator.XXXXXX" 2>/dev/null)
      if [ -n "$ERN_ZTMP" ]; then
        ERN_ZDOTDIR="${ZDOTDIR:-$HOME}"
        export ERN_ZTMP ERN_ZDOTDIR
        for f in zshenv zprofile zshrc zlogin; do
          {
            printf '%s\n' 'ZDOTDIR="$ERN_ZDOTDIR"'
            printf '%s\n' "[ -r \"\$ZDOTDIR/.$f\" ] && . \"\$ZDOTDIR/.$f\""
            case "$f" in
              zshrc)
                printf '%s\n' '__termigator_osc7() { printf "\033]7;file://%s%s\007" "$HOST" "$PWD"; }'
                printf '%s\n' 'precmd_functions+=(__termigator_osc7)'
                printf '%s\n' 'ZDOTDIR="$ERN_ZTMP"'
                ;;
              zlogin)
                printf '%s\n' 'rm -rf "$ERN_ZTMP"; unset ERN_ZTMP ERN_ZDOTDIR'
                ;;
              *)
                printf '%s\n' 'ZDOTDIR="$ERN_ZTMP"'
                ;;
            esac
          } > "$ERN_ZTMP/.$f"
        done
        ZDOTDIR="$ERN_ZTMP"
        export ZDOTDIR
        exec "$ern_sh" -l
      fi
      ;;
    fish)
      exec "$ern_sh" -l -C 'function __termigator_osc7 --on-event fish_prompt; printf "\033]7;file://%s%s\007" $hostname $PWD; end'
      ;;
    sh|dash|ash|ksh|mksh)
      # Their prompt is re-expanded each time: ENV (read after the profile) prefixes PS1 with the report.
      ERN_ENVF=$(mktemp "${TMPDIR:-/tmp}/termigator.XXXXXX" 2>/dev/null)
      if [ -n "$ERN_ENVF" ]; then
        {
          printf '%s\n' '[ -n "$ERN_ENV" ] && [ -r "$ERN_ENV" ] && . "$ERN_ENV"'
          printf '%s\n' 'PS1='"'"'$(printf "\033]7;file://%s%s\007" "$(hostname)" "$PWD")'"'"'"$PS1"'
          printf '%s\n' 'rm -f "$ERN_ENVF"; unset ERN_ENVF ERN_ENV'
        } > "$ERN_ENVF"
        ERN_ENV="${ENV:-}"
        ENV="$ERN_ENVF"
        export ENV ERN_ENV ERN_ENVF
        exec "$ern_sh" -l
      fi
      ;;
  esac
fi
