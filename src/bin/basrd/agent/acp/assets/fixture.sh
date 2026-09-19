# Minimal ACP peer for process ownership and protocol integration tests.
printf '%s\n' "$$" > "$BASR_FIXTURE_PID"
if [ "$BASR_FIXTURE_MODE" = hang_start ]; then
    sleep 60
    exit
fi
while IFS= read -r request; do
    id=$(printf '%s' "$request" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*)
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":true,"audio":true,"embeddedContext":true}}}}\n' "$id"
            ;;
        *'"method":"session/new"'*)
            if [ "$BASR_FIXTURE_MODE" = early_update ]; then
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"config_option_update","configOptions":[{"id":"enabled","name":"Enabled","type":"boolean","currentValue":true}]}}}'
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"current_mode_update","currentModeId":"code"}}}'
            fi
            printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"fixture","configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"fast","options":[{"value":"fast","name":"Fast"},{"value":"deep","name":"Deep"}]}],"modes":{"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"},{"id":"code","name":"Code"}]},"_meta":{"fixture":"session"}}}\n' "$id"
            if [ "$BASR_FIXTURE_MODE" = idle_exit ]; then sleep 0.1; exit; fi
            ;;
        *'"method":"session/prompt"'*)
            prompt_id=$id
            printf '%s\n' "$request" > "$BASR_FIXTURE_PID.request"
            printf started > "$BASR_FIXTURE_PID.prompt"
            if [ "$BASR_FIXTURE_MODE" = wait_release ]; then
                while [ ! -f "$BASR_FIXTURE_PID.release" ]; do sleep 0.01; done
            fi
            if [ "$BASR_FIXTURE_MODE" = prompt_error ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"fixture prompt failed"}}\n' "$id"
            elif [ "$BASR_FIXTURE_MODE" != wait_cancel ] && [ "$BASR_FIXTURE_MODE" != ignore_cancel ]; then
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"thinking"}}}}'
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"answer"}}}}'
                if [ "$BASR_FIXTURE_MODE" = rich ]; then
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"image","data":"aW1hZ2U=","mimeType":"image/png","_meta":{"image":"metadata"}}}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"resource","resource":{"uri":"file:///report.txt","text":"file contents","mimeType":"text/plain"}}}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"tool_call","toolCallId":"read","title":"read file","kind":"read","locations":[{"path":"/tmp/report.txt","line":7}],"content":[{"type":"content","content":{"type":"text","text":"tool output"}}]}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"tool_call_update","toolCallId":"read","status":"completed"}}}'
                fi
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn","_meta":{"fixture":"response"}}}\n' "$id"
            fi
            ;;
        *'"method":"session/set_config_option"'*)
            printf '%s\n' "$request" > "$BASR_FIXTURE_PID.config"
            case "$request" in
                *'"value":"invalid"'*)
                    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"invalid config value","data":{"configId":"model"}}}\n' "$id"
                    ;;
                *)
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"deep","options":[{"value":"fast","name":"Fast"},{"value":"deep","name":"Deep"}]}],"_meta":{"fixture":"config"}}}\n' "$id"
                    ;;
            esac
            ;;
        *'"method":"session/set_mode"'*)
            printf '%s\n' "$request" > "$BASR_FIXTURE_PID.mode"
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            ;;
        *'"method":"session/cancel"'*)
            printf '%s\n' "$request" > "$BASR_FIXTURE_PID.cancel"
            if [ -n "$prompt_id" ] && [ "$BASR_FIXTURE_MODE" != ignore_cancel ]; then
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"cancelled"}}\n' "$prompt_id"
            fi
            ;;
    esac
done
