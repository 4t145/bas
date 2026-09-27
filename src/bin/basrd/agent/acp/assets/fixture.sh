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
            if [ "$BASR_FIXTURE_MODE" = auth_session ] || [ "$BASR_FIXTURE_MODE" = auth_data ]; then
                case "$request" in *'"auth":{"terminal":true}'*) ;; *) exit 1;; esac
                printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"authMethods":[{"type":"terminal","id":"login","name":"Sign in","args":["auth","login"],"env":{"AUTH_MODE":"fixture"}}],"agentCapabilities":{}}}\n' "$id"
            else
                if [ "$BASR_FIXTURE_MODE" = load ] || [ "$BASR_FIXTURE_MODE" = hang_load ] || [ "$BASR_FIXTURE_MODE" = reject_load ] || [ "$BASR_FIXTURE_MODE" = context ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}\n' "$id"
                elif [ "$BASR_FIXTURE_MODE" = resume ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"resume":{}}}}}\n' "$id"
                else
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":true,"audio":true,"embeddedContext":true}}}}\n' "$id"
                fi
            fi
            ;;
        *'"method":"session/load"'*)
            printf '%s\n' "$request" > "$BASR_FIXTURE_PID.load"
            if [ "$BASR_FIXTURE_MODE" = hang_load ]; then
                while [ ! -f "$BASR_FIXTURE_PID.release" ]; do sleep 0.01; done
            fi
            if [ "$BASR_FIXTURE_MODE" = reject_load ] || { [ "$BASR_FIXTURE_MODE" = context ] && [ ! -f "$BASR_FIXTURE_PID.context" ]; }; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"unknown session"}}\n' "$id"
            else
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"replayed"}}}}'
                printf '{"jsonrpc":"2.0","id":%s,"result":{"modes":{"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"}]},"_meta":{"fixture":"loaded"}}}\n' "$id"
            fi
            ;;
        *'"method":"session/resume"'*)
            printf '%s\n' "$request" > "$BASR_FIXTURE_PID.resume"
            printf '{"jsonrpc":"2.0","id":%s,"result":{"modes":{"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"}]},"_meta":{"fixture":"resumed"}}}\n' "$id"
            ;;
        *'"method":"session/new"'*)
            printf '%s\n' "$request" > "$BASR_FIXTURE_PID.new"
            if [ "$BASR_FIXTURE_MODE" = context ]; then printf '0\n' > "$BASR_FIXTURE_PID.context"; fi
            if [ "$BASR_FIXTURE_MODE" = reject_session ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"session rejected SECRET"}}\n' "$id"
                continue
            fi
            if [ "$BASR_FIXTURE_MODE" = auth_session ] || [ "$BASR_FIXTURE_MODE" = auth_data ]; then
                if [ "$BASR_FIXTURE_MODE" = auth_data ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"Sign in required","data":{"authMethods":[{"type":"terminal","id":"specific","name":"Use specific login","args":["login"]}],"reason":"expired"}}}\n' "$id"
                else
                    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"Sign in required"}}\n' "$id"
                fi
                continue
            fi
            if [ "$BASR_FIXTURE_MODE" = probe_access ]; then
                printf '{"jsonrpc":"2.0","id":"write","method":"fs/write_text_file","params":{"sessionId":"fixture","path":"%s.write","content":"forbidden"}}\n' "$BASR_FIXTURE_PID"
                IFS= read -r response
                case "$response" in *'"error"'*) ;; *) exit 1;; esac
                printf '{"jsonrpc":"2.0","id":"read","method":"fs/read_text_file","params":{"sessionId":"fixture","path":"%s"}}\n' "$BASR_FIXTURE_PID"
                IFS= read -r response
                case "$response" in *'"error"'*) ;; *) exit 1;; esac
                printf '%s\n' '{"jsonrpc":"2.0","id":"permission","method":"session/request_permission","params":{"sessionId":"fixture","toolCall":{"toolCallId":"probe","title":"probe","kind":"execute","status":"pending"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]}}'
                IFS= read -r response
                case "$response" in *'"cancelled"'*) ;; *) exit 1;; esac
                printf '%s\n' '{"jsonrpc":"2.0","id":"terminal","method":"terminal/create","params":{"sessionId":"fixture","command":"touch","args":["forbidden"]}}'
                IFS= read -r response
                case "$response" in *'"error"'*) ;; *) exit 1;; esac
            fi
            if [ "$BASR_FIXTURE_MODE" = early_update ]; then
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"config_option_update","configOptions":[{"id":"enabled","name":"Enabled","type":"boolean","currentValue":true}]}}}'
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"current_mode_update","currentModeId":"code"}}}'
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"startup"}}}}'
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
            if [ "$BASR_FIXTURE_MODE" = prompt_disconnect ]; then
                exit
            elif [ "$BASR_FIXTURE_MODE" = malformed_prompt ]; then
                printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            elif [ "$BASR_FIXTURE_MODE" = corrupt_prompt ]; then
                printf '%s\n' 'not json'
            elif [ "$BASR_FIXTURE_MODE" = prompt_error ]; then
                printf '\n   \n'
                printf '%s\n' '{"jsonrpc":"2.0","method":"fixture/unknown","params":{}}'
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"fixture prompt rejected"}}\n' "$id"
                BASR_FIXTURE_MODE=normal
            elif [ "$BASR_FIXTURE_MODE" = context ]; then
                previous=$(cat "$BASR_FIXTURE_PID.context")
                count=$((previous + 1))
                printf '%s\n' "$count" > "$BASR_FIXTURE_PID.context"
                printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"turn %s after %s"}}}}\n' "$count" "$previous"
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
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
                    config_value=deep
                    case "$request" in *'"value":"fast"'*) config_value=fast;; esac
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"%s","options":[{"value":"fast","name":"Fast"},{"value":"deep","name":"Deep"}]}],"_meta":{"fixture":"config"}}}\n' "$id" "$config_value"
                    if [ "$BASR_FIXTURE_MODE" = config_notification ]; then
                        printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"config_option_update","configOptions":[{"id":"enabled","name":"Enabled","type":"boolean","currentValue":true}]}}}'
                    fi
                    ;;
            esac
            ;;
        *'"method":"session/set_mode"'*)
            printf '%s\n' "$request" > "$BASR_FIXTURE_PID.mode"
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            if [ "$BASR_FIXTURE_MODE" = mode_notification ]; then
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"current_mode_update","currentModeId":"code"}}}'
            fi
            ;;
        *'"method":"session/cancel"'*)
            printf '%s\n' "$request" > "$BASR_FIXTURE_PID.cancel"
            if [ -n "$prompt_id" ] && [ "$BASR_FIXTURE_MODE" != ignore_cancel ]; then
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"cancelled"}}\n' "$prompt_id"
            fi
            ;;
    esac
done
