# rust-terminal: marcas OSC 133 (bloques de comandos) y OSC 7 (directorio).
#
#   A  empieza el prompt          (precmd, el último de la lista)
#   B  empieza la entrada         (zle-line-init, tras dibujar el prompt)
#   C  empieza la salida          (preexec, con el comando en cmdline_url)
#   D  termina, con su código     (PROMPT_EOL_MARK o precmd, solo si hubo C)
#
# Los hooks se colocan en el primer prompt, después de cargar .zshrc, para
# quedar detrás de temas como Powerlevel10k que reescriben el prompt.

typeset -gi __rt_cmd_running=0
typeset -gi __rt_last_status=0

# Codifica $1 con %XX (byte a byte) en REPLY.
__rt_urlencode() {
  emulate -L zsh
  local LC_ALL=C
  local s=$1 out= c hex
  local -i i
  for (( i = 1; i <= ${#s}; i++ )); do
    c=${s[i]}
    if [[ $c == [a-zA-Z0-9._~/-] ]]; then
      out+=$c
    else
      printf -v hex '%%%02X' "'$c"
      out+=${hex[-3,-1]}
    fi
  done
  REPLY=$out
}

# Primero de precmd: guarda el código de salida antes de que otro hook lo pise.
__rt_save_status() {
  __rt_last_status=$?
}

# Con PROMPT_SP, zsh rellena la línea tras una salida sin salto de línea
# (la marca `%`) *antes* de llamar a precmd. Para que el bloque termine justo
# tras la salida, la marca D va dentro de PROMPT_EOL_MARK mientras dura el
# comando (con `%?` como código) y después se restaura el valor del usuario.
# Efecto secundario: un comando que lea $PROMPT_EOL_MARK ve el valor modificado.
typeset -gi __rt_d_in_eol=0
typeset -gi __rt_eol_was_set=0
typeset -g __rt_saved_eol_mark=

# Último de precmd: cierra el bloque anterior y marca el nuevo prompt.
# Sin `emulate -L`: debe ver las opciones reales del usuario.
__rt_precmd() {
  if (( __rt_cmd_running )); then
    if (( __rt_d_in_eol )); then
      if (( __rt_eol_was_set )); then
        PROMPT_EOL_MARK=$__rt_saved_eol_mark
      else
        builtin unset PROMPT_EOL_MARK
      fi
      __rt_d_in_eol=0
    else
      builtin print -rn -- $'\e]133;D;'"$__rt_last_status"$'\a'
    fi
    __rt_cmd_running=0
  fi
  __rt_urlencode "$PWD"
  builtin print -rn -- $'\e]7;file://'"${HOST}${REPLY}"$'\a\e]133;A\a'
}

__rt_preexec() {
  __rt_urlencode "$1"
  builtin print -rn -- $'\e]133;C;cmdline_url='"$REPLY"$'\a'
  __rt_cmd_running=1
  if [[ -o prompt_sp && -o prompt_cr ]]; then
    __rt_eol_was_set=${+PROMPT_EOL_MARK}
    __rt_saved_eol_mark=${PROMPT_EOL_MARK-%B%S%#%s%b}
    PROMPT_EOL_MARK=$'%{\e]133;D;%?\a%}'"$__rt_saved_eol_mark"
    __rt_d_in_eol=1
  fi
}

__rt_line_init() {
  builtin print -rn -- $'\e]133;B\a'
}

# `claude` lanzado a mano: se le pasan los hooks de esta terminal para que el
# agente salga en la sidebar. Solo en uso interactivo; un subcomando (`mcp`,
# `update`...), `-p`, `--bg`, `--settings` propio... va tal cual.
__rt_claude() {
  emulate -L zsh
  local plain=0 a
  if [[ ! -r ${RT_CLAUDE_SETTINGS-} ]]; then
    plain=1
  elif (( $# )) && [[ $1 == (agents|attach|auth|auto-mode|doctor|gateway|import|install|logs|mcp|plugin|plugins|purge|respawn|rm|setup-token|stop|kill|ultrareview|update|upgrade) ]]; then
    plain=1
  else
    for a in "$@"; do
      [[ $a == (-p|--print|--bg|-h|--help|-v|--version|--settings|--settings=*) ]] && { plain=1; break; }
    done
  fi
  if (( plain )); then
    command claude "$@"
  else
    command claude --settings "$RT_CLAUDE_SETTINGS" "$@"
  fi
}

# Selección con ⇧← / ⇧→ (RT_SHIFT_SELECT=1). `__rt_sel` vale 1 mientras la
# región activa la ha creado esta integración; las regiones del usuario
# (Ctrl+Espacio) no se tocan.
typeset -gi __rt_sel=0

__rt_shift_select() {
  (( REGION_ACTIVE )) || zle set-mark-command
  __rt_sel=1
  case $WIDGET in
    __rt-select-left) zle backward-char ;;
    __rt-select-right) zle forward-char ;;
    __rt-select-word-left) zle backward-word ;;
    __rt-select-word-right) zle forward-word ;;
    __rt-select-home) zle beginning-of-line ;;
    __rt-select-end) zle end-of-line ;;
  esac
}

# Borra el texto seleccionado sin pasar por el kill ring.
__rt_sel_delete() {
  local a=$(( MARK < CURSOR ? MARK : CURSOR ))
  local b=$(( MARK < CURSOR ? CURSOR : MARK ))
  BUFFER="${BUFFER[1,a]}${BUFFER[b+1,-1]}"
  CURSOR=$a
  REGION_ACTIVE=0
  __rt_sel=0
}

# Teclear, ⌫ o Supr con una selección propia la reemplazan o la borran; si no
# la hay, se delega en el widget que hubiera antes.
__rt_sel_wrapper() {
  if (( __rt_sel && REGION_ACTIVE )); then
    __rt_sel_delete
    [[ $WIDGET == self-insert ]] || return 0
  fi
  __rt_sel=0
  zle "__rt_orig_$WIDGET" -- "$@"
}

# Cualquier otro widget (mover el cursor, Esc, Intro…) quita la selección.
__rt_sel_redraw() {
  (( __rt_sel )) || return 0
  [[ $LASTWIDGET == __rt-select-* ]] && return 0
  REGION_ACTIVE=0
  __rt_sel=0
}

__rt_shift_select_init() {
  emulate -L zsh
  builtin autoload -Uz add-zle-hook-widget
  local name seq map w
  local -A keys=(
    __rt-select-left       '^[[1;2D'
    __rt-select-right      '^[[1;2C'
    __rt-select-word-left  '^[[1;4D'
    __rt-select-word-right '^[[1;4C'
    __rt-select-home       '^[[1;10D'
    __rt-select-end        '^[[1;10C'
  )
  for name seq in ${(kv)keys}; do
    zle -N $name __rt_shift_select
    for map in emacs viins; do
      # No se pisa un enlace del usuario.
      [[ $(builtin bindkey -M $map -- $seq 2>/dev/null) == *undefined-key* ]] \
        && builtin bindkey -M $map -- $seq $name
    done
  done
  # ⇧Inicio / ⇧Fin hacen lo mismo que ⌘⇧←/→.
  for map in emacs viins; do
    [[ $(builtin bindkey -M $map -- '^[[1;2H' 2>/dev/null) == *undefined-key* ]] \
      && builtin bindkey -M $map -- '^[[1;2H' __rt-select-home
    [[ $(builtin bindkey -M $map -- '^[[1;2F' 2>/dev/null) == *undefined-key* ]] \
      && builtin bindkey -M $map -- '^[[1;2F' __rt-select-end
  done
  # Se envuelve lo que haya ahora (puede ser el de un plugin).
  for w in self-insert backward-delete-char delete-char; do
    zle -A $w "__rt_orig_$w" 2>/dev/null || continue
    zle -N $w __rt_sel_wrapper
  done
  add-zle-hook-widget zle-line-pre-redraw __rt_sel_redraw
}

__rt_deferred_init() {
  emulate -L zsh
  precmd_functions=(__rt_save_status ${precmd_functions:#(__rt_deferred_init|__rt_save_status|__rt_precmd)} __rt_precmd)
  preexec_functions+=(__rt_preexec)
  # Sin pisar un `claude` del usuario (función o alias definidos en .zshrc).
  if [[ -n ${RT_CLAUDE_SETTINGS-} ]] && (( ! ${+functions[claude]} && ! ${+aliases[claude]} )); then
    functions[claude]='__rt_claude "$@"'
  fi
  builtin autoload -Uz add-zle-hook-widget
  add-zle-hook-widget zle-line-init __rt_line_init
  [[ -n ${RT_SHIFT_SELECT-} ]] && __rt_shift_select_init
  # Este primer prompt: los hooks recién colocados aún no han corrido.
  __rt_precmd
}

precmd_functions+=(__rt_deferred_init)
