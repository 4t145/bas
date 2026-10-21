printf '%s\n' "$$" > "$BAS_FIXTURE_PID"
while IFS= read -r request; do
    id=$(printf '%s' "$request" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*)
            printf 'initialize\n' >> "$BAS_FIXTURE_PID.events"
            printf '%s\n' '{"jsonrpc":"2.0","id":"warm-auth","method":"elicitation/create","params":{"mode":"form","message":"must not prompt","requestedSchema":{"type":"object","properties":{}}}}'
            IFS= read -r response || exit 0
            printf '%s\n' "$response" > "$BAS_FIXTURE_PID.denied"
            printf '%s\n' '{"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"not-pending"}}'
            printf '%s\n' '{"jsonrpc":"2.0","method":"future/notification","params":{}}'
            printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"phantom","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"must not replay"}}}}'
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"futureInitialize":17,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"close":{}}}}}\n' "$id"
            ;;
        *'"method":"session/new"'*)
            printf 'new\n' >> "$BAS_FIXTURE_PID.events"
            if [ "$BAS_FIXTURE_MODE" = reject ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"create failed"}}\n' "$id"
            else
                printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"fixture"}}\n' "$id"
            fi
            ;;
        *'"method":"session/load"'*)
            printf 'load\n' >> "$BAS_FIXTURE_PID.events"
            printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"replayed"}}}}'
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            ;;
        *'"method":"session/close"'*)
            printf 'close\n' >> "$BAS_FIXTURE_PID.events"
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            ;;
    esac
done
