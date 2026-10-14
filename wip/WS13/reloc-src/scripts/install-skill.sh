#!/usr/bin/env bash
#
# scripts/install-skill.sh — install the cgg skills into supported agents.
#
# When to run (manual, end-user-facing):
#   - On a fresh checkout, after building/installing `cgg`, to teach
#     your coding agent how to use it.
#   - After updating skills/*/SKILL.md if you want existing installs
#     refreshed (re-runs are idempotent; pass --force to overwrite).
#   - NEVER invoked automatically — this writes to the user's agent
#     config, which is out of scope for any commit-time hook.
#
# Discovers every skill under skills/*/SKILL.md, detects Claude Code,
# Kiro, Cline, Roo Code, and OpenCode, asks each detected agent
# (once) for scope (global vs project) and a target path, then
# installs every discovered skill into that agent's idiomatic
# location and format.
#
# Safety:
#   - Never overwrites existing different content without --force.
#   - Never edits VS Code settings.json (Cline/Roo global) — prints
#     manual instructions instead.
#   - AGENTS.md (OpenCode) is updated between per-skill
#     <!-- <name>-skill:begin --> / <!-- <name>-skill:end --> markers,
#     so other content is preserved.
#
# Usage:
#   scripts/install-skill.sh                # interactive
#   scripts/install-skill.sh --dry-run      # show actions, write nothing
#   scripts/install-skill.sh --force        # overwrite without prompting
#   scripts/install-skill.sh --yes          # accept all defaults
#   scripts/install-skill.sh --only NAME    # install only one skill (repeatable)
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SKILLS_DIR="$REPO_ROOT/skills"

DRY_RUN=0
FORCE=0
ASSUME_YES=0
ONLY=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) DRY_RUN=1; shift ;;
        --force)   FORCE=1; shift ;;
        --yes|-y)  ASSUME_YES=1; shift ;;
        --only)    ONLY+=("$2"); shift 2 ;;
        -h|--help)
            sed -n '3,30p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *)
            echo "unknown flag: $1" >&2; exit 2 ;;
    esac
done

# Discover skills (each is a dir under skills/ containing SKILL.md).
SKILL_NAMES=()
for d in "$SKILLS_DIR"/*/; do
    [[ -f "$d/SKILL.md" ]] || continue
    name="$(basename "$d")"
    if [[ ${#ONLY[@]} -gt 0 ]]; then
        for o in "${ONLY[@]}"; do [[ "$o" == "$name" ]] && SKILL_NAMES+=("$name"); done
    else
        SKILL_NAMES+=("$name")
    fi
done

if [[ ${#SKILL_NAMES[@]} -eq 0 ]]; then
    echo "error: no skills found under $SKILLS_DIR" >&2
    exit 1
fi

# ---------- helpers ----------

c_bold()  { printf "\033[1m%s\033[0m" "$*"; }
c_dim()   { printf "\033[2m%s\033[0m" "$*"; }
c_green() { printf "\033[32m%s\033[0m" "$*"; }
c_yellow(){ printf "\033[33m%s\033[0m" "$*"; }
c_red()   { printf "\033[31m%s\033[0m" "$*"; }

# All informational output goes to stderr so command substitution
# captures only the return value.
say() { printf "%s\n" "$*" >&2; }
note(){ printf "  %s %s\n" "$(c_dim '·')" "$*" >&2; }
ok()  { printf "  %s %s\n" "$(c_green '✓')" "$*" >&2; }
warn(){ printf "  %s %s\n" "$(c_yellow '!')" "$*" >&2; }
err() { printf "  %s %s\n" "$(c_red '✗')" "$*" >&2; }

prompt() {
    local q="$1" def="${2:-}" ans=""
    if [[ $ASSUME_YES -eq 1 ]]; then
        printf "%s\n" "$def"
        return
    fi
    if [[ -n "$def" ]]; then
        read -r -p "$q [$def]: " ans </dev/tty || true
    else
        read -r -p "$q: " ans </dev/tty || true
    fi
    printf "%s\n" "${ans:-$def}"
}

confirm() {
    local q="$1" def="${2:-n}" ans=""
    if [[ $ASSUME_YES -eq 1 ]]; then
        [[ "$def" == "y" ]]
        return
    fi
    local hint="[y/N]"; [[ "$def" == "y" ]] && hint="[Y/n]"
    read -r -p "$q $hint " ans </dev/tty || true
    ans="${ans:-$def}"
    [[ "${ans,,}" == "y" || "${ans,,}" == "yes" ]]
}

strip_frontmatter() {
    awk '
        BEGIN { in_fm=0 }
        NR==1 && /^---[[:space:]]*$/ { in_fm=1; next }
        in_fm && /^---[[:space:]]*$/ { in_fm=0; next }
        in_fm { next }
        { print }
    ' "$1"
}

extract_description() {
    awk '
        BEGIN { in_fm=0 }
        NR==1 && /^---[[:space:]]*$/ { in_fm=1; next }
        in_fm && /^---[[:space:]]*$/ { exit }
        in_fm && /^description:[[:space:]]*/ {
            sub(/^description:[[:space:]]*/, "")
            print
            exit
        }
    ' "$1"
}

write_file() {
    local dest="$1"
    if [[ $DRY_RUN -eq 1 ]]; then
        local bytes
        bytes=$(cat | wc -c)
        ok "would write $dest ($bytes bytes)"
        return
    fi
    mkdir -p "$(dirname "$dest")"
    cat > "$dest"
    ok "wrote $dest"
}

install_verbatim() {
    # install_verbatim <agent> <skill_src> <dest>
    local agent="$1" src="$2" dest="$3"
    if [[ -f "$dest" ]]; then
        if cmp -s "$src" "$dest"; then
            ok "$agent: already up to date ($dest)"
            return
        fi
        if [[ $FORCE -eq 0 ]]; then
            warn "$agent: $dest exists and differs"
            if ! confirm "    overwrite?"; then
                note "skipped"
                return
            fi
        fi
    fi
    cat "$src" | write_file "$dest"
}

install_stripped() {
    # install_stripped <agent> <skill_name> <skill_src> <dest>
    local agent="$1" name="$2" src="$3" dest="$4"
    local body desc rendered
    desc="$(extract_description "$src")"
    body="$(strip_frontmatter "$src")"
    rendered="$(printf "# Skill: %s\n\n_When to use:_ %s\n\n%s\n" "$name" "$desc" "$body")"
    if [[ -f "$dest" ]]; then
        if [[ "$(cat "$dest")" == "$rendered" ]]; then
            ok "$agent: already up to date ($dest)"
            return
        fi
        if [[ $FORCE -eq 0 ]]; then
            warn "$agent: $dest exists and differs"
            if ! confirm "    overwrite?"; then
                note "skipped"
                return
            fi
        fi
    fi
    printf "%s" "$rendered" | write_file "$dest"
}

install_agentsmd() {
    # install_agentsmd <agent> <skill_name> <skill_src> <dest>
    # Appends/updates a section between per-skill markers in AGENTS.md.
    local agent="$1" name="$2" src="$3" dest="$4"
    local desc body section begin end
    desc="$(extract_description "$src")"
    body="$(strip_frontmatter "$src")"
    begin="<!-- ${name}-skill:begin -->"
    end="<!-- ${name}-skill:end -->"
    section="$(printf "%s\n# Skill: %s\n\n_When to use:_ %s\n\n%s\n%s" "$begin" "$name" "$desc" "$body" "$end")"

    if [[ ! -f "$dest" ]]; then
        printf "%s\n" "$section" | write_file "$dest"
        return
    fi

    if grep -qF "$begin" "$dest"; then
        local tmp; tmp="$(mktemp)"
        SECTION="$section" BEGIN_M="$begin" END_M="$end" awk '
            $0 == ENVIRON["BEGIN_M"] { print ENVIRON["SECTION"]; skip=1; next }
            $0 == ENVIRON["END_M"]   { skip=0; next }
            !skip { print }
        ' "$dest" > "$tmp"
        if cmp -s "$tmp" "$dest"; then
            rm -f "$tmp"
            ok "$agent: $name already up to date in $dest"
            return
        fi
        if [[ $DRY_RUN -eq 1 ]]; then
            rm -f "$tmp"
            ok "$agent: would update $name block in $dest"
            return
        fi
        mv "$tmp" "$dest"
        ok "$agent: updated $name block in $dest"
        return
    fi

    if [[ $DRY_RUN -eq 1 ]]; then
        ok "$agent: would append $name block to $dest"
        return
    fi
    printf "\n\n%s\n" "$section" >> "$dest"
    ok "$agent: appended $name block to $dest"
}

# ---------- detection ----------

detect_claude()   { command -v claude >/dev/null 2>&1 || [[ -d "$HOME/.claude" ]]; }
detect_kiro()     { command -v kiro   >/dev/null 2>&1 || [[ -d "$HOME/.kiro"   ]]; }
detect_opencode() { command -v opencode >/dev/null 2>&1 || [[ -d "$HOME/.config/opencode" ]]; }

vscode_ext_dirs() {
    local d
    for d in "$HOME/.vscode/extensions" \
             "$HOME/.vscode-server/extensions" \
             "$HOME/.cursor/extensions" \
             "$HOME/.windsurf/extensions"; do
        [[ -d "$d" ]] && echo "$d"
    done
}
detect_cline() {
    local d
    for d in $(vscode_ext_dirs); do
        compgen -G "$d/saoudrizwan.claude-dev-*" >/dev/null 2>&1 && return 0
    done
    return 1
}
detect_roo() {
    local d
    for d in $(vscode_ext_dirs); do
        compgen -G "$d/rooveterinaryinc.roo-cline-*" >/dev/null 2>&1 && return 0
        compgen -G "$d/rooveterinaryinc.roo-code-*"  >/dev/null 2>&1 && return 0
    done
    return 1
}

# ---------- scope/path prompts ----------

ask_scope() {
    local label="$1" ans
    say ""
    say "$(c_bold "$label") detected."
    if [[ $ASSUME_YES -eq 1 ]]; then echo "project"; return; fi
    while :; do
        read -r -p "  install [g]lobal, [p]roject, or [s]kip? " ans </dev/tty || true
        case "${ans,,}" in
            g|global)  echo "global";  return ;;
            p|project) echo "project"; return ;;
            s|skip|"") echo "skip";    return ;;
            *) say "    answer g, p, or s" ;;
        esac
    done
}

ask_project_dir() {
    local def="$PWD" dir
    dir="$(prompt "  project root" "$def")"
    dir="${dir/#\~/$HOME}"
    if [[ ! -d "$dir" ]]; then
        warn "directory does not exist: $dir"
        if confirm "  create it?"; then
            [[ $DRY_RUN -eq 0 ]] && mkdir -p "$dir"
        else
            echo ""
            return
        fi
    fi
    printf "%s\n" "$dir"
}

# ---------- per-agent flows ----------
#
# Each flow installs ALL discovered skills at the chosen scope.

flow_claude() {
    detect_claude || { note "Claude Code: not detected"; return; }
    local scope; scope="$(ask_scope "Claude Code")"
    [[ "$scope" == "skip" ]] && { note "skipped"; return; }
    local root=""
    if [[ "$scope" == "global" ]]; then root="$HOME/.claude"
    else local d; d="$(ask_project_dir)"; [[ -z "$d" ]] && return; root="$d/.claude"
    fi
    for name in "${SKILL_NAMES[@]}"; do
        install_verbatim "claude:$name" "$SKILLS_DIR/$name/SKILL.md" "$root/skills/$name/SKILL.md"
    done
}

flow_kiro() {
    detect_kiro || { note "Kiro: not detected"; return; }
    local scope; scope="$(ask_scope "Kiro")"
    [[ "$scope" == "skip" ]] && { note "skipped"; return; }
    local root=""
    if [[ "$scope" == "global" ]]; then root="$HOME/.kiro"
    else local d; d="$(ask_project_dir)"; [[ -z "$d" ]] && return; root="$d/.kiro"
    fi
    for name in "${SKILL_NAMES[@]}"; do
        install_verbatim "kiro:$name" "$SKILLS_DIR/$name/SKILL.md" "$root/steering/$name.md"
    done
}

flow_cline() {
    detect_cline || { note "Cline: not detected"; return; }
    local scope; scope="$(ask_scope "Cline")"
    case "$scope" in
        global)
            warn "Cline global = VS Code setting 'cline.customInstructions'."
            warn "This installer will not edit settings.json. Paste these files manually:"
            for name in "${SKILL_NAMES[@]}"; do warn "  $SKILLS_DIR/$name/SKILL.md"; done
            ;;
        project)
            local d; d="$(ask_project_dir)"; [[ -z "$d" ]] && return
            for name in "${SKILL_NAMES[@]}"; do
                install_stripped "cline:$name" "$name" "$SKILLS_DIR/$name/SKILL.md" "$d/.clinerules/$name.md"
            done ;;
        skip) note "skipped" ;;
    esac
}

flow_roo() {
    detect_roo || { note "Roo Code: not detected"; return; }
    local scope; scope="$(ask_scope "Roo Code")"
    case "$scope" in
        global)
            warn "Roo Code global = VS Code setting (custom instructions)."
            warn "This installer will not edit settings.json. Paste these files manually:"
            for name in "${SKILL_NAMES[@]}"; do warn "  $SKILLS_DIR/$name/SKILL.md"; done
            ;;
        project)
            local d; d="$(ask_project_dir)"; [[ -z "$d" ]] && return
            for name in "${SKILL_NAMES[@]}"; do
                install_stripped "roo:$name" "$name" "$SKILLS_DIR/$name/SKILL.md" "$d/.roo/rules/$name.md"
            done ;;
        skip) note "skipped" ;;
    esac
}

flow_opencode() {
    detect_opencode || { note "OpenCode: not detected"; return; }
    local scope; scope="$(ask_scope "OpenCode")"
    [[ "$scope" == "skip" ]] && { note "skipped"; return; }
    local dest=""
    if [[ "$scope" == "global" ]]; then dest="$HOME/.config/opencode/AGENTS.md"
    else local d; d="$(ask_project_dir)"; [[ -z "$d" ]] && return; dest="$d/AGENTS.md"
    fi
    for name in "${SKILL_NAMES[@]}"; do
        install_agentsmd "opencode:$name" "$name" "$SKILLS_DIR/$name/SKILL.md" "$dest"
    done
}

# ---------- main ----------

say "$(c_bold "cgg skill installer")"
say "  skills: ${SKILL_NAMES[*]}"
[[ $DRY_RUN -eq 1 ]] && say "  $(c_yellow 'dry-run — no files will be written')"
[[ $FORCE   -eq 1 ]] && say "  $(c_yellow 'force — existing files will be overwritten without prompting')"

flow_claude
flow_kiro
flow_cline
flow_roo
flow_opencode

say ""
say "done."
