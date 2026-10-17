# A controlled ACP peer. Logout must not create a session or invoke authentication.
while IFS= read -r request; do
    printf '%s\n' "$request" >> "$BAS_FIXTURE_REQUESTS"
    id=$(printf '%s' "$request" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$request" in
        *'"method":"initialize"'*)
            if [ "$BAS_FIXTURE_MODE" = unsupported ]; then caps='{}'; else caps='{"auth":{"logout":{}}}'; fi
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":%s}}\n' "$id" "$caps"
            ;;
        *'"method":"logout"'*)
            if [ "$BAS_FIXTURE_MODE" = reject ]; then
                printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"logout rejected","data":{"reason":"fixture"}}}\n' "$id"
            elif [ "$BAS_FIXTURE_MODE" = hang ]; then
                sleep 60
            else
                printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
            fi
            ;;
        *)
            printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"unexpected method"}}\n' "$id"
            ;;
    esac
done
