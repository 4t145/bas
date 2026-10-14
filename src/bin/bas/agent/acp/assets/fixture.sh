# Minimal ACP peer for process ownership and protocol integration tests.
read_response() {
    # Closing the transport is not an empty response from the client.
    IFS= read -r response || exit 0
}

printf '%s\n' "$$" > "$BAS_FIXTURE_PID"
if [ "$BAS_FIXTURE_MODE" = hang_start ]; then
    sleep 60
    exit
fi
while IFS= read -r request; do
    id=$(printf '%s' "$request" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*)
            if [ "$BAS_FIXTURE_MODE" = rich ]; then
                printf '%s\n' "$request" > "$BAS_FIXTURE_PID.initialize"
            fi
            if [ "${BAS_FIXTURE_MODE#agent_auth}" != "$BAS_FIXTURE_MODE" ]; then
                printf '%s\n' "$request" > "$BAS_FIXTURE_PID.initialize"
                if [ "$BAS_FIXTURE_MODE" = agent_auth_rpc_initialize_reject ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"Internal error","data":{"message":"Provider initialization failed","credentials":"private initialization credentials"}}}\n' "$id"
                    continue
                fi
                if [ "$BAS_FIXTURE_MODE" = agent_auth_terminal ] || [ "$BAS_FIXTURE_MODE" = agent_auth_terminal_ready ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"authMethods":[{"type":"terminal","id":"login","name":"Sign in","args":["login"]}],"agentCapabilities":{}}}\n' "$id"
                elif [ "$BAS_FIXTURE_MODE" = agent_auth_unknown ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"authMethods":[{"type":"future_mode","id":"login","name":"Future setup","futureField":{"kept":true}}],"agentCapabilities":{},"futureInitialize":17}}\n' "$id"
                else
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"authMethods":[{"id":"login","name":"Sign in","_meta":{"command":"ignored"}}],"agentCapabilities":{}}}\n' "$id"
                fi
            elif [ "$BAS_FIXTURE_MODE" = auth_session ] || [ "$BAS_FIXTURE_MODE" = auth_data ]; then
                case "$request" in *'"auth":{"terminal":true}'*) ;; *) exit 1;; esac
                printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"authMethods":[{"type":"terminal","id":"login","name":"Sign in","args":["auth","login"],"env":{"AUTH_MODE":"fixture"}}],"agentCapabilities":{}}}\n' "$id"
            else
                if [ "$BAS_FIXTURE_MODE" = load ] || [ "$BAS_FIXTURE_MODE" = hang_load ] || [ "$BAS_FIXTURE_MODE" = reject_load ] || [ "$BAS_FIXTURE_MODE" = context ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}\n' "$id"
                elif [ "$BAS_FIXTURE_MODE" = resume ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"resume":{}}}}}\n' "$id"
                else
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":true,"audio":true,"embeddedContext":true}}}}\n' "$id"
                fi
            fi
            ;;
        *'"method":"authenticate"'*)
            printf '%s\n' "$request" > "$BAS_FIXTURE_PID.authenticate"
            if [ "$BAS_FIXTURE_MODE" = agent_auth_reject ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"authentication rejected SECRET"}}\n' "$id"
            elif [ "$BAS_FIXTURE_MODE" = agent_auth_rpc_reject ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"Internal error","data":{"message":"Provider rejected this authentication method","access_token":"private access credential","refresh_token":"private refresh credential","env":{"API_KEY":"private environment"},"stderr":"private stderr","stack":"private stack"}}}\n' "$id"
            elif [ "$BAS_FIXTURE_MODE" = agent_auth_rpc_reject_details ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"Internal error","data":{"details":"Could not save authentication settings","credentials":"private credentials","stack":"private stack"}}}\n' "$id"
            elif [ "$BAS_FIXTURE_MODE" = agent_auth_rpc_reject_string ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"Internal error","data":"Provider rejected authentication: refresh_token=private-refresh Authorization: Bearer private-bearer at https://user:password@provider.example/callback?code=private-code&state=private-state"}}\n' "$id"
            elif [ "$BAS_FIXTURE_MODE" = agent_auth_hang ]; then
                sleep 60 &
                printf '%s\n' "$!" > "$BAS_FIXTURE_PID.auth-child"
                wait
            elif [ "$BAS_FIXTURE_MODE" = agent_auth_form ] || [ "$BAS_FIXTURE_MODE" = agent_auth_url ] || [ "$BAS_FIXTURE_MODE" = agent_auth_form_wrong_scope ] || [ "$BAS_FIXTURE_MODE" = agent_auth_form_cancel ] || [ "$BAS_FIXTURE_MODE" = agent_auth_form_wrong_session ]; then
                auth_id=$id
                if [ "$BAS_FIXTURE_MODE" = agent_auth_url ]; then
                    printf '{"jsonrpc":"2.0","id":"elicit","method":"elicitation/create","params":{"mode":"url","requestId":%s,"message":"Authorize the fixture","elicitationId":"opaque/url:17","url":"https://example.com/authorize"}}\n' "$auth_id"
                elif [ "$BAS_FIXTURE_MODE" = agent_auth_form_wrong_session ]; then
                    printf '%s\n' '{"jsonrpc":"2.0","id":"elicit","method":"elicitation/create","params":{"mode":"form","sessionId":"unrelated","message":"Choose project","requestedSchema":{"type":"object","properties":{"project":{"type":"string"}},"required":["project"]}}}'
                else
                    scope_id=$auth_id
                    if [ "$BAS_FIXTURE_MODE" = agent_auth_form_wrong_scope ]; then scope_id='"not-live"'; fi
                    printf '{"jsonrpc":"2.0","id":"elicit","method":"elicitation/create","params":{"mode":"form","requestId":%s,"message":"Choose a project","requestedSchema":{"type":"object","properties":{"project":{"type":"string","minLength":1}},"required":["project"]},"_meta":{"futureHint":"preserved"}}}\n' "$scope_id"
                fi
                if [ "$BAS_FIXTURE_MODE" = agent_auth_form_cancel ]; then
                    printf '%s\n' '{"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"elicit"}}'
                fi
                read_response
                printf '%s\n' "$response" > "$BAS_FIXTURE_PID.elicitation"
                case "$response" in
                    *'"code":-32800'*)
                        authenticated=yes
                        printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$auth_id"
                        ;;
                    *'"action":"accept"'*)
                        authenticated=yes
                        if [ "$BAS_FIXTURE_MODE" = agent_auth_url ]; then
                            printf '%s\n' '{"jsonrpc":"2.0","method":"elicitation/complete","params":{"elicitationId":"unknown"}}'
                            printf '%s\n' '{"jsonrpc":"2.0","method":"elicitation/complete","params":{"elicitationId":"opaque/url:17","_meta":{"future":"preserved"}}}'
                        fi
                        printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$auth_id"
                        ;;
                    *) printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"setup cancelled"}}\n' "$auth_id" ;;
                esac
            else
                authenticated=yes
                printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            fi
            ;;
        *'"method":"session/load"'*)
            printf '%s\n' "$request" > "$BAS_FIXTURE_PID.load"
            if [ "$BAS_FIXTURE_MODE" = hang_load ]; then
                while [ ! -f "$BAS_FIXTURE_PID.release" ]; do sleep 0.01; done
            fi
            if [ "$BAS_FIXTURE_MODE" = reject_load ] || { [ "$BAS_FIXTURE_MODE" = context ] && [ ! -f "$BAS_FIXTURE_PID.context" ]; }; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"unknown session"}}\n' "$id"
            else
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"replayed"}}}}'
                printf '{"jsonrpc":"2.0","id":%s,"result":{"modes":{"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"}]},"_meta":{"fixture":"loaded"}}}\n' "$id"
            fi
            ;;
        *'"method":"session/resume"'*)
            printf '%s\n' "$request" > "$BAS_FIXTURE_PID.resume"
            printf '{"jsonrpc":"2.0","id":%s,"result":{"modes":{"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"}]},"_meta":{"fixture":"resumed"}}}\n' "$id"
            ;;
        *'"method":"session/new"'*)
            printf '%s\n' "$request" > "$BAS_FIXTURE_PID.new"
            if [ "$BAS_FIXTURE_MODE" = agent_auth_rpc_discovery_reject ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"Provider configuration rejected","data":{"message":"Provider project is missing","env":"private project environment"}}}\n' "$id"
                continue
            fi
            if [ "$BAS_FIXTURE_MODE" = agent_auth_rpc_after_reject ] && [ "$authenticated" = yes ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"Internal error","data":{"message":"Provider cannot create a session","stderr":"private session stderr"}}}\n' "$id"
                continue
            fi
            if [ "$BAS_FIXTURE_MODE" = agent_auth_strict ]; then
                case "$request" in
                    *'"mcpServers":[]'*) ;;
                    *) printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"mcpServers must be an array"}}\n' "$id"; continue ;;
                esac
                session_new_count=$((${session_new_count:-0} + 1))
                printf '%s\n' "$request" > "$BAS_FIXTURE_PID.new.$session_new_count"
            fi
            if [ "${BAS_FIXTURE_MODE#agent_auth}" != "$BAS_FIXTURE_MODE" ] && { [ "$authenticated" != yes ] || [ "$BAS_FIXTURE_MODE" = agent_auth_still_required ]; } && [ ! -f "$BAS_FIXTURE_PID.ready" ]; then
                if [ "$BAS_FIXTURE_MODE" = agent_auth_unknown ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"Setup required","futureError":{"preserved":true}}}\n' "$id"
                else
                    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"Sign in required"}}\n' "$id"
                fi
                continue
            fi
            if [ "$BAS_FIXTURE_MODE" = context ]; then printf '0\n' > "$BAS_FIXTURE_PID.context"; fi
            if [ "$BAS_FIXTURE_MODE" = reject_session ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"session rejected SECRET"}}\n' "$id"
                continue
            fi
            if [ "$BAS_FIXTURE_MODE" = auth_session ] || [ "$BAS_FIXTURE_MODE" = auth_data ]; then
                if [ "$BAS_FIXTURE_MODE" = auth_data ]; then
                    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"Sign in required","data":{"authMethods":[{"type":"terminal","id":"specific","name":"Use specific login","args":["login"]}],"reason":"expired"},"futureError":{"preserved":true}}}\n' "$id"
                else
                    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"Sign in required"}}\n' "$id"
                fi
                continue
            fi
            if [ "$BAS_FIXTURE_MODE" = probe_access ] || [ "$BAS_FIXTURE_MODE" = agent_auth_access ]; then
                printf '{"jsonrpc":"2.0","id":"write","method":"fs/write_text_file","params":{"sessionId":"fixture","path":"%s.write","content":"forbidden"}}\n' "$BAS_FIXTURE_PID"
                read_response
                case "$response" in *'"error"'*) ;; *) exit 1;; esac
                printf '{"jsonrpc":"2.0","id":"read","method":"fs/read_text_file","params":{"sessionId":"fixture","path":"%s"}}\n' "$BAS_FIXTURE_PID"
                read_response
                case "$response" in *'"error"'*) ;; *) exit 1;; esac
                printf '%s\n' '{"jsonrpc":"2.0","id":"permission","method":"session/request_permission","params":{"sessionId":"fixture","toolCall":{"toolCallId":"probe","title":"probe","kind":"execute","status":"pending"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]}}'
                read_response
                case "$response" in *'"cancelled"'*) ;; *) exit 1;; esac
                printf '%s\n' '{"jsonrpc":"2.0","id":"terminal","method":"terminal/create","params":{"sessionId":"fixture","command":"touch","args":["forbidden"]}}'
                read_response
                case "$response" in *'"error"'*) ;; *) exit 1;; esac
            fi
            if [ "$BAS_FIXTURE_MODE" = early_update ]; then
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"config_option_update","configOptions":[{"id":"enabled","name":"Enabled","type":"boolean","currentValue":true}]}}}'
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"current_mode_update","currentModeId":"code"}}}'
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"startup"}}}}'
            fi
            printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"fixture","configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"fast","options":[{"value":"fast","name":"Fast"},{"value":"deep","name":"Deep"}]}],"modes":{"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"},{"id":"code","name":"Code"}]},"_meta":{"fixture":"session"}}}\n' "$id"
            if [ "$BAS_FIXTURE_MODE" = idle_exit ]; then sleep 0.1; exit; fi
            ;;
        *'"method":"session/prompt"'*)
            prompt_id=$id
            printf '%s\n' "$request" > "$BAS_FIXTURE_PID.request"
            printf started > "$BAS_FIXTURE_PID.prompt"
            if [ "$BAS_FIXTURE_MODE" = wait_release ]; then
                while [ ! -f "$BAS_FIXTURE_PID.release" ]; do sleep 0.01; done
            fi
            if [ "$BAS_FIXTURE_MODE" = prompt_disconnect ]; then
                exit
            elif [ "$BAS_FIXTURE_MODE" = malformed_prompt ]; then
                printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            elif [ "$BAS_FIXTURE_MODE" = corrupt_prompt ]; then
                printf '%s\n' 'not json'
            elif [ "$BAS_FIXTURE_MODE" = prompt_error ]; then
                printf '\n   \n'
                printf '%s\n' '{"jsonrpc":"2.0","method":"fixture/unknown","params":{}}'
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"fixture prompt rejected","data":{"field":"prompt","retryable":true}}}\n' "$id"
                BAS_FIXTURE_MODE=normal
            elif [ "$BAS_FIXTURE_MODE" = interactive_permission ]; then
                printf '%s\n' '{"jsonrpc":"2.0","id":"permission","method":"session/request_permission","params":{"sessionId":"fixture","toolCall":{"toolCallId":"execute","title":"run command","kind":"execute","status":"pending","rawInput":{"command":"pwd"}},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"reject-once","name":"Reject once","kind":"reject_once"}]}}'
                read_response
                printf '%s\n' "$response" > "$BAS_FIXTURE_PID.permission"
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$prompt_id"
            elif [ "$BAS_FIXTURE_MODE" = permission_cancel_successor ]; then
                printf '%s\n' '{"jsonrpc":"2.0","id":"permission-old","method":"session/request_permission","params":{"sessionId":"fixture","toolCall":{"toolCallId":"execute","title":"old command","status":"pending"},"options":[{"optionId":"allow-old","name":"Allow old","kind":"allow_once"}]}}'
                while [ ! -f "$BAS_FIXTURE_PID.cancel" ]; do sleep 0.01; done
                printf '%s\n' '{"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"permission-old"}}'
                printf '%s\n' '{"jsonrpc":"2.0","id":"permission-successor","method":"session/request_permission","params":{"sessionId":"fixture","toolCall":{"toolCallId":"execute","title":"new command","status":"pending"},"options":[{"optionId":"allow-successor","name":"Allow new","kind":"allow_once"}]}}'
                replies=0
                while [ "$replies" -lt 2 ] && read_response; do
                    case "$response" in
                        *'"id":"permission-old"'*) printf '%s\n' "$response" > "$BAS_FIXTURE_PID.permission-cancelled" ;;
                        *'"id":"permission-successor"'*) printf '%s\n' "$response" > "$BAS_FIXTURE_PID.permission-successor" ;;
                        *) continue ;;
                    esac
                    replies=$((replies + 1))
                done
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$prompt_id"
            elif [ "$BAS_FIXTURE_MODE" = session_async_elicitation ]; then
                printf '%s\n' '{"jsonrpc":"2.0","id":"async-form","method":"elicitation/create","params":{"mode":"form","sessionId":"fixture","toolCallId":"question-item","message":"请选择工作方式","requestedSchema":{"type":"object","properties":{"workflow":{"type":"string","title":"工作方式","description":"希望先做哪一步？","oneOf":[{"const":"inspect","title":"检查","description":"先检查现有代码"},{"const":"implement","title":"实现","description":"直接实现功能"}]},"workflow_note":{"type":"string","title":"补充说明"}},"required":["workflow"]}}}'
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$prompt_id"
                while read_response; do
                    case "$response" in *'"id":"async-form"'*) break;; esac
                done
                printf '%s\n' "$response" > "$BAS_FIXTURE_PID.elicitation"
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Received async answer"}}}}'
            elif [ "$BAS_FIXTURE_MODE" = session_elicitation ]; then
                printf '%s\n' '{"jsonrpc":"2.0","id":"session-form","method":"elicitation/create","params":{"mode":"form","sessionId":"fixture","message":"Choose a project","requestedSchema":{"type":"object","properties":{"project":{"type":"string"}},"required":["project"]},"_meta":{"future":"preserved"}}}'
                while read_response; do
                    case "$response" in *'"method":"session/cancel"'*) continue;; esac
                    break
                done
                printf '%s\n' "$response" > "$BAS_FIXTURE_PID.elicitation"
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$prompt_id"
            elif [ "$BAS_FIXTURE_MODE" = context ]; then
                previous=$(cat "$BAS_FIXTURE_PID.context")
                count=$((previous + 1))
                printf '%s\n' "$count" > "$BAS_FIXTURE_PID.context"
                printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"turn %s after %s"}}}}\n' "$count" "$previous"
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
            elif [ "$BAS_FIXTURE_MODE" != wait_cancel ] && [ "$BAS_FIXTURE_MODE" != ignore_cancel ]; then
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"thinking"}}}}'
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"answer"}}}}'
                if [ "$BAS_FIXTURE_MODE" = rich ]; then
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"image","data":"aW1hZ2U=","mimeType":"image/png","_meta":{"image":"metadata"}}}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"resource","resource":{"uri":"file:///report.txt","text":"file contents","mimeType":"text/plain"}}}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"tool_call","toolCallId":"read","title":"read file","kind":"read","locations":[{"path":"/tmp/report.txt","line":7}],"content":[{"type":"content","content":{"type":"text","text":"tool output"}}]}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"tool_call_update","toolCallId":"read","status":"completed"}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"plan_update","plan":{"type":"items","planId":"fixture-plan","entries":[{"content":"Inspect output","priority":"high","status":"in_progress"}]},"_meta":{"fixture":"plan"}}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"compaction_summary_chunk","compactionId":"fixture-compaction","content":{"type":"text","text":"Retained context"}}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"compaction_update","compactionId":"fixture-compaction","status":"completed","error":null,"_meta":{"fixture":"compaction"}}}}'
                    printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"notice","severity":"warning","title":"Provider retry","description":"The upstream request will be retried"}}}'
                fi
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn","_meta":{"fixture":"response"}}}\n' "$id"
            fi
            ;;
        *'"method":"session/set_config_option"'*)
            printf '%s\n' "$request" > "$BAS_FIXTURE_PID.config"
            if [ "$BAS_FIXTURE_MODE" = auth_operation ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"Sign in required","data":{"reason":"expired"}}}\n' "$id"
                continue
            fi
            case "$request" in
                *'"value":"invalid"'*)
                    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"invalid config value","data":{"configId":"model"}}}\n' "$id"
                    ;;
                *)
                    config_value=deep
                    case "$request" in *'"value":"fast"'*) config_value=fast;; esac
                    printf '{"jsonrpc":"2.0","id":%s,"result":{"configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"%s","options":[{"value":"fast","name":"Fast"},{"value":"deep","name":"Deep"}]}],"_meta":{"fixture":"config"}}}\n' "$id" "$config_value"
                    if [ "$BAS_FIXTURE_MODE" = config_notification ]; then
                        printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"config_option_update","configOptions":[{"id":"enabled","name":"Enabled","type":"boolean","currentValue":true}]}}}'
                    fi
                    ;;
            esac
            ;;
        *'"method":"session/set_mode"'*)
            printf '%s\n' "$request" > "$BAS_FIXTURE_PID.mode"
            if [ "$BAS_FIXTURE_MODE" = auth_operation ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"Sign in required","data":{"reason":"expired"}}}\n' "$id"
                continue
            fi
            printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            if [ "$BAS_FIXTURE_MODE" = mode_notification ]; then
                printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"current_mode_update","currentModeId":"code"}}}'
            fi
            ;;
        *'"method":"session/cancel"'*)
            printf '%s\n' "$request" > "$BAS_FIXTURE_PID.cancel"
            if [ -n "$prompt_id" ] && [ "$BAS_FIXTURE_MODE" != ignore_cancel ]; then
                printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"cancelled"}}\n' "$prompt_id"
            fi
            ;;
    esac
done
