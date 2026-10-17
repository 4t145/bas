while IFS= read -r request; do
    printf '%s\n' "$request" >> "$BAS_FIXTURE_REQUESTS"
    id=$(printf '%s' "$request" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*)
            caps='"loadSession":true,"sessionCapabilities":{"additionalDirectories":{}}'
            case "$BAS_FIXTURE_MODE" in
                unsupported) caps='"loadSession":true';;
                resume) caps='"sessionCapabilities":{"additionalDirectories":{},"resume":{}}';;
            esac
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{%s}}}\n' "$id" "$caps"
            ;;
        *'"method":"session/new"'*)
            printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"same-native-id"}}\n' "$id"
            ;;
        *'"method":"session/load"'*|*'"method":"session/resume"'*)
            case "$BAS_FIXTURE_MODE" in
                reject) printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"roots rejected"}}\n' "$id"; continue;;
                hang) sleep 60;;
                slow) sleep 0.2;;
            esac
            if [ "$BAS_FIXTURE_MODE" != resume ]; then
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"same-native-id","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"replayed, not new"}}}}'
            fi
            printf '{"jsonrpc":"2.0","id":%s,"result":{"configOptions":[]}}\n' "$id"
            printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"same-native-id","update":{"sessionUpdate":"session_info_update","title":"after reload"}}}'
            ;;
        *'"method":"session/prompt"'*)
            if [ "$BAS_FIXTURE_MODE" = working ]; then sleep 60; fi
            printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
            ;;
    esac
done
