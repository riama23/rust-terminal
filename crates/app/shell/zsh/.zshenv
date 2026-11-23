# rust-terminal: integración con zsh (se inyecta mediante ZDOTDIR).
# Restaura el ZDOTDIR original, carga el .zshenv del usuario y, en sesiones
# interactivas, activa las marcas de bloques. No debe imprimir nada.
if [[ -n "${RT_ORIG_ZDOTDIR+x}" ]]; then
  ZDOTDIR="$RT_ORIG_ZDOTDIR"
  unset RT_ORIG_ZDOTDIR
else
  unset ZDOTDIR
fi
typeset -g __rt_integration_dir="${${(%):-%x}:A:h}"
if [[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]]; then
  builtin source "${ZDOTDIR:-$HOME}/.zshenv"
fi
if [[ -o interactive ]]; then
  builtin source "$__rt_integration_dir/rt-integration.zsh"
fi
