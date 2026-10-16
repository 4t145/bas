# Agent-owned durable history fixture. Pagination state belongs to this process.
printf '%s\n' "$$" > "$BAS_FIXTURE_PID"
while IFS= read -r request; do
    id=$(printf '%s' "$request" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*)
            if [ "$BAS_FIXTURE_MODE" = unsupported ]; then
                capabilities='{}'
            elif [ "$BAS_FIXTURE_MODE" = resume ]; then
                capabilities='{"sessionCapabilities":{"list":{},"resume":{},"close":{},"delete":{}}}'
            else
                capabilities='{"loadSession":true,"sessionCapabilities":{"list":{},"close":{},"delete":{}}}'
            fi
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":%s}}\n' "$id" "$capabilities"
            ;;
        *'"method":"session/list"'*)
            printf '%s\n' "$request" >> "$BAS_FIXTURE_PID.list"
            if [ "$BAS_FIXTURE_MODE" = list_hang ]; then
                sleep 60
            elif [ "$listed" = yes ]; then
                if [ "$BAS_FIXTURE_MODE" = repeat_cursor ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"sessions":[],"nextCursor":"opaque/cursor:2"}}\n' "$id"
                else
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"sessions":[]}}\n' "$id"
                fi
            else
                listed=yes
                if [ -f "$BAS_FIXTURE_PID.deleted" ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"sessions":[]}}\n' "$id"
                else
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"sessions":[{"sessionId":"fixture","cwd":"%s","title":"External history","updatedAt":"2026-10-16T10:00:00Z","_meta":{"origin":"native-cli"}}],"nextCursor":"opaque/cursor:2"}}\n' "$id" "$BAS_FIXTURE_WORKSPACE"
                fi
            fi
            ;;
        *'"method":"session/new"'*)
            printf '%s\n' "$request" >> "$BAS_FIXTURE_PID.new"
            printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"fixture"}}\n' "$id"
            ;;
        *'"method":"session/load"'*)
            printf '%s\n' "$request" >> "$BAS_FIXTURE_PID.load"
            if [ "$BAS_FIXTURE_MODE" = reject_load ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"unknown session"}}\n' "$id"
            else
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"user_message_chunk","messageId":"user-1","content":{"type":"text","text":"past question"}}}}'
                count=0
                while [ "$count" -lt 12 ]; do
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","messageId":"reply-1","content":{"type":"text","text":"past reply"}}}}'
                    count=$((count + 1))
                done
                printf '{"jsonrpc":"2.0","id":%s,"result":{"configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"history-model","options":[{"value":"history-model","name":"History model"}]}]}}\n' "$id"
            fi
            ;;
        *'"method":"session/resume"'*)
            printf '%s\n' "$request" >> "$BAS_FIXTURE_PID.resume"
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            ;;
        *'"method":"session/prompt"'*)
            printf '%s\n' "$request" >> "$BAS_FIXTURE_PID.prompt"
            printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","messageId":"reply-2","content":{"type":"text","text":"continued"}}}}'
            printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
            ;;
        *'"method":"session/close"'*)
            printf '%s\n' "$request" >> "$BAS_FIXTURE_PID.close"
            if [ "$BAS_FIXTURE_MODE" = reject_close ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"closure rejected"}}\n' "$id"
            elif [ "$BAS_FIXTURE_MODE" = close_hang ]; then
                sleep 60
            else
                printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            fi
            ;;
        *'"method":"session/delete"'*)
            printf '%s\n' "$request" >> "$BAS_FIXTURE_PID.delete"
            if [ "$BAS_FIXTURE_MODE" = reject_delete ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"deletion rejected"}}\n' "$id"
            else
                printf 'deleted\n' > "$BAS_FIXTURE_PID.deleted"
                printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            fi
            ;;
        *)
            printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"not implemented"}}\n' "$id"
            ;;
    esac
done
