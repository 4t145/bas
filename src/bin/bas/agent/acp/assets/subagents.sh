announce() {
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"root","update":{"sessionUpdate":"subagent_update","sessionId":"child","title":"Researcher","description":"Inspect the project","state":{"state":"%s"}}}}\n' "$1"
    printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"child","update":{"sessionUpdate":"subagent_update","sessionId":"nested","title":"Nested worker","state":{"state":"%s"}}}}\n' "$1"
}
child_text() {
    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"nested","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"child transcript, not the parent answer"}}}}'
}
while IFS= read -r request; do
    printf '%s\n' "$request" >> "$BAS_FIXTURE_REQUESTS"
    id=$(printf '%s' "$request" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*)
            case "$request" in
                *'"subagents":{}'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"additionalDirectories":{}}}}}\n' "$id";;
                *) printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"missing subagents declaration"}}\n' "$id";;
            esac
            ;;
        *'"method":"session/new"'*)
            printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"root"}}\n' "$id"
            ;;
        *'"method":"session/load"'*)
            announce idle
            child_text
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            ;;
        *'"method":"session/prompt"'*)
            case "$request" in
                *finish*) announce idle;;
                *)
                    announce running
                    child_text
                    printf '{"jsonrpc":"2.0","id":"child-read","method":"fs/read_text_file","params":{"sessionId":"nested","path":"%s"}}\n' "$BAS_FIXTURE_FILE"
                    printf '{"jsonrpc":"2.0","id":"foreign-read","method":"fs/read_text_file","params":{"sessionId":"unannounced","path":"%s"}}\n' "$BAS_FIXTURE_FILE"
                    ;;
            esac
            printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
            ;;
    esac
done
