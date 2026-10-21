record() {
    printf '%s %s %s %s\n' "$BAS_RESTART_RUN" "$BAS_RESTART_AGENT" "$1" "$2" >> "$BAS_RESTART_LOG"
}
while IFS= read -r request; do
    id=$(printf '%s' "$request" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*)
            record initialize
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}\n' "$id"
            ;;
        *'"method":"session/new"'*)
            record new "raw-$$"
            printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"raw-%s"}}\n' "$id" "$$"
            ;;
        *'"method":"session/load"'*)
            session=$(printf '%s' "$request" | sed -n 's/.*"sessionId":"\([^"]*\)".*/\1/p')
            record load "$session"
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            ;;
        *'"method":"session/prompt"'*)
            record prompt
            printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
            ;;
    esac
done
