#!/bin/bash
# This script enhances the auto-generated zsh completion to support
# completing sandboxed commands by invoking the command's real completer
# but getting filesystem context from inside the sandbox.

set -e

input_file="$1"
output_file="$2"

if [ -z "$output_file" ]; then
    echo "Usage: $0 <input_file> <output_file>"
    exit 1
fi

# Create enhanced completion with sandboxed command support
cat > "$output_file" << 'COMPLETION_SCRIPT'
#compdef sandbox

# Original clap-generated completion function (renamed)
function _clap_dynamic_completer_sandbox_base() {
    local _CLAP_COMPLETE_INDEX=$(expr $CURRENT - 1)
    local _CLAP_IFS=$'\n'

    local completions=("${(@f)$( \
        _CLAP_IFS="$_CLAP_IFS" \
        _CLAP_COMPLETE_INDEX="$_CLAP_COMPLETE_INDEX" \
        COMPLETE="zsh" \
        sandbox -- ${words} 2>/dev/null \
    )}")

    if [[ -n $completions ]]; then
        # Separate tilde-prefixed completions to avoid shell escaping the ~
        local -a tilde_completions regular_completions
        for c in $completions; do
            if [[ "$c" == '~'* ]]; then
                tilde_completions+=("$c")
            else
                regular_completions+=("$c")
            fi
        done
        if [[ -n $regular_completions ]]; then
            _describe 'values' regular_completions
        fi
        if [[ -n $tilde_completions ]]; then
            compadd -Q -a tilde_completions
        fi
    fi
}

# Helper: Get file completions from inside the sandbox
# Arguments: $1 = sandbox flags (e.g., "--name=foo"), $2 = prefix to complete, $3 = "dirs" for directories only
_sandbox_file_completions() {
    local sandbox_flags="$1"
    local prefix="$2"
    local type="$3"

    if [[ "$type" == "dirs" ]]; then
        sandbox ${=sandbox_flags} zsh -c "
            setopt nullglob extendedglob
            local p='${prefix//\'/\'\\\'\'}'
            [[ -z \"\$p\" ]] && p='./'
            print -l \"\${p}\"*(/N)
        " 2>/dev/null
    else
        sandbox ${=sandbox_flags} zsh -c "
            setopt nullglob extendedglob
            local p='${prefix//\'/\'\\\'\'}'
            [[ -z \"\$p\" ]] && p=''
            if [[ -n \"\$p\" ]]; then
                print -l \"\${p}\"*(N)
            else
                print -l *(N)
            fi
        " 2>/dev/null
    fi
}

# Global variable to hold sandbox flags during completion
typeset -g _sandbox_completion_flags=""

# Override _files to use sandbox when completing sandboxed commands
_sandbox_files() {
    local current_word="$1"
    local completions_raw
    completions_raw=$(_sandbox_file_completions "$_sandbox_completion_flags" "$current_word" "files")

    if [[ -n "$completions_raw" ]]; then
        local -a completions expl
        completions=(${(f)completions_raw})
        _wanted files expl file compadd -a completions
        return 0
    fi
    return 1
}

_sandbox_directories() {
    local current_word="$1"
    local completions_raw
    completions_raw=$(_sandbox_file_completions "$_sandbox_completion_flags" "$current_word" "dirs")

    if [[ -n "$completions_raw" ]]; then
        local -a completions expl
        completions=(${(f)completions_raw})
        _wanted directories expl directory compadd -a completions
        return 0
    fi
    return 1
}

# Enhanced completion function with sandboxed command support
function _clap_dynamic_completer_sandbox() {
    local _CLAP_COMPLETE_INDEX=$(expr $CURRENT - 1)

    # List of sandbox actions (subcommands)
    local -a sandbox_actions
    sandbox_actions=(accept reject status diff list stop delete config sync)

    # Parse words to find sandbox flags and the first non-flag argument
    local first_non_flag=""
    local is_action=0
    local found_command_position=0
    local -a sandbox_flags_array
    sandbox_flags_array=()

    # Iterate through words to find first non-flag after 'sandbox'
    # and collect any sandbox flags (--name, --storage-dir, etc.)
    for ((i = 2; i <= ${#words[@]}; i++)); do
        local word="${words[$i]}"
        if [[ "$word" =~ ^- ]]; then
            # It's a flag - collect it for passing to sandbox
            sandbox_flags_array+=("$word")
            # If it's a flag that takes a value and doesn't use =, grab the next word too
            if [[ "$word" =~ ^--(name|storage-dir|net|bind|mask|config)$ && $((i+1)) -le ${#words[@]} ]]; then
                ((i++))
                sandbox_flags_array+=("${words[$i]}")
            fi
        else
            first_non_flag="$word"
            found_command_position=$i
            break
        fi
    done

    # Build the sandbox flags string
    local sandbox_flags="${sandbox_flags_array[*]}"

    # Check if the first non-flag word is a sandbox action
    for action in $sandbox_actions; do
        if [[ "$first_non_flag" == "$action" ]]; then
            is_action=1
            break
        fi
    done

    # If we're completing arguments to a sandboxed command (not an action)
    if [[ $is_action -eq 0 && -n "$first_non_flag" && $CURRENT -gt $found_command_position ]]; then
        # We're completing arguments to a sandboxed command!
        local sandboxed_cmd="$first_non_flag"
        local current_word="${words[$CURRENT]}"

        # Check if sandbox exists (pass through the flags!)
        if ! sandbox ${=sandbox_flags} config sandbox_dir &>/dev/null 2>&1; then
            # Sandbox doesn't exist - fall back to normal completion
            # Try to invoke the command's completer
            local completer="_${sandboxed_cmd}"
            if (( $+functions[$completer] )); then
                # Shift words to remove 'sandbox' and its flags
                local -a orig_words
                orig_words=("${words[@]}")
                local orig_current=$CURRENT

                # Reconstruct words as if sandbox wasn't there
                words=("${words[@]:$found_command_position-1}")
                CURRENT=$((orig_current - found_command_position + 1))

                # Invoke the real completer
                $completer
                local ret=$?

                # Restore
                words=("${orig_words[@]}")
                CURRENT=$orig_current
                return $ret
            else
                _files
                return $?
            fi
        fi

        # Sandbox exists - we need to complete with sandbox filesystem context
        # Strategy: Invoke the command's completer, but override file completion

        # Store sandbox flags globally so our overridden functions can use them
        _sandbox_completion_flags="$sandbox_flags"

        local completer="_${sandboxed_cmd}"
        if (( $+functions[$completer] )); then
            # Save original file completion functions
            local orig_files
            local orig_path_files
            local orig_directories
            if (( $+functions[_files] )); then
                orig_files="${functions[_files]}"
            fi
            if (( $+functions[_path_files] )); then
                orig_path_files="${functions[_path_files]}"
            fi
            if (( $+functions[_directories] )); then
                orig_directories="${functions[_directories]}"
            fi

            # Override file completion to use sandbox
            _files() {
                local current="${words[$CURRENT]}"
                _sandbox_files "$current"
            }

            _path_files() {
                local current="${words[$CURRENT]}"
                _sandbox_files "$current"
            }

            _directories() {
                local current="${words[$CURRENT]}"
                _sandbox_directories "$current"
            }

            # Shift context to the sandboxed command
            local -a orig_words
            orig_words=("${words[@]}")
            local orig_current=$CURRENT

            # Remove 'sandbox' and its flags from words
            words=("${words[@]:$found_command_position-1}")
            CURRENT=$((orig_current - found_command_position + 1))

            # Invoke the real completer
            $completer
            local ret=$?

            # Restore everything
            words=("${orig_words[@]}")
            CURRENT=$orig_current
            _sandbox_completion_flags=""

            if [[ -n "$orig_files" ]]; then
                functions[_files]="$orig_files"
            else
                unfunction _files 2>/dev/null
            fi
            if [[ -n "$orig_path_files" ]]; then
                functions[_path_files]="$orig_path_files"
            else
                unfunction _path_files 2>/dev/null
            fi
            if [[ -n "$orig_directories" ]]; then
                functions[_directories]="$orig_directories"
            else
                unfunction _directories 2>/dev/null
            fi

            return $ret
        else
            # No specific completer - just do sandbox file completion
            _sandbox_files "$current_word"
            _sandbox_completion_flags=""
            return $?
        fi
    fi

    # For everything else (actions, flags, command names), use clap's completion
    _clap_dynamic_completer_sandbox_base
}

# Ensure functions are available before registering
if (( ! $+functions[_clap_dynamic_completer_sandbox] )); then
    # This shouldn't happen, but just in case
    return 1
fi

compdef _clap_dynamic_completer_sandbox sandbox
COMPLETION_SCRIPT

echo "Enhanced zsh completion written to $output_file"
