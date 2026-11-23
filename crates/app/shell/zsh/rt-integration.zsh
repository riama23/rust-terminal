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

__rt_deferred_init() {
  emulate -L zsh
  precmd_functions=(__rt_save_status ${precmd_functions:#(__rt_deferred_init|__rt_save_status|__rt_precmd)} __rt_precmd)
  preexec_functions+=(__rt_preexec)
  builtin autoload -Uz add-zle-hook-widget
  add-zle-hook-widget zle-line-init __rt_line_init
  # Este primer prompt: los hooks recién colocados aún no han corrido.
  __rt_precmd
}

precmd_functions+=(__rt_deferred_init)
