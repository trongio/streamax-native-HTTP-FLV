# Backend HMAC URL signing — design note

Live-stream URLs minted by `system/php/request/streamax_stream.php` previously
embedded a `hash` parameter generated with `generateRandomString(rand(20,200))`
— random, never validated. Anyone with a `devid` could stream forever.

The PHP side now generates a real HMAC-SHA256 over a TTL-bound payload. This is
**forward-compatible**: the camera server still ignores the hash today, so
existing clients keep working. When the camera server is updated to validate,
the signing infrastructure is already in place.

## PHP change

```diff
- $hash = generateRandomString(rand(20,200));
+ $streamax_secret = getenv("STREAMAX_STREAM_SECRET")
+     ?: (defined("STREAMAX_STREAM_SECRET") ? STREAMAX_STREAM_SECRET : "");
+ $expires = time() + 3600;                                // 1-hour TTL
+ $payload = "{$uniqueId}|{$chl}|{$st}|{$audio}|{$expires}";
+ $hash = $streamax_secret !== ""
+     ? hash_hmac("sha256", $payload, $streamax_secret)
+     : generateRandomString(rand(20, 200));               // back-compat
```

URL format changes from

```
.../live.flv?devid=X&chl=1&st=1&audio=0&hash=<99 random chars>
```

to

```
.../live.flv?devid=X&chl=1&st=1&audio=0&expires=1715789432&hash=<sha256 hex>
```

## Provisioning the secret

Add to `../config.php` (one level up from the WWW root):

```php
define("STREAMAX_STREAM_SECRET", "<64-char random hex>");
```

Generate the key once with:

```bash
openssl rand -hex 32
```

Same secret must be configured on the camera server.

## Camera server validation (the missing half)

The Streamax media server at `YOUR-CAMERA-HOST.example.com:22060` needs a small middleware
that, before serving `/live.flv`, validates:

1. `expires > current_unix_time` (URL hasn't expired)
2. `hash == hmac_sha256(secret, "{devid}|{chl}|{st}|{audio}|{expires}")`

In nginx with `lua-resty-string`:

```nginx
location /live.flv {
    access_by_lua_block {
        local args = ngx.req.get_uri_args()
        local payload = args.devid .. "|" .. args.chl .. "|" .. args.st
                     .. "|" .. args.audio .. "|" .. args.expires
        local hmac = ngx.hmac_sha256(os.getenv("STREAMAX_STREAM_SECRET"), payload)
        local hex  = require("resty.string").to_hex(hmac)
        if hex ~= args.hash or tonumber(args.expires) < ngx.time() then
            ngx.exit(ngx.HTTP_FORBIDDEN)
        end
    }
    proxy_pass http://upstream_streamax;
}
```

In Go (if the camera server is custom):

```go
func validate(r *http.Request) bool {
    q := r.URL.Query()
    expires, err := strconv.ParseInt(q.Get("expires"), 10, 64)
    if err != nil || time.Now().Unix() > expires { return false }
    payload := q.Get("devid") + "|" + q.Get("chl") + "|" + q.Get("st") +
               "|" + q.Get("audio") + "|" + q.Get("expires")
    mac := hmac.New(sha256.New, []byte(os.Getenv("STREAMAX_STREAM_SECRET")))
    mac.Write([]byte(payload))
    return hmac.Equal([]byte(q.Get("hash")), []byte(hex.EncodeToString(mac.Sum(nil))))
}
```

## Native client impact

None. The native players in this repo treat the URL as opaque — they call the
PHP backend's `request/dashcam_getstream`, get a URL, hand it to the player.
Whether the URL is signed with a real HMAC or fake random data is invisible to
them.

When the camera server starts validating, no client code changes are needed.
Clients that cache URLs longer than 1 hour will start getting 403s — fine; they
should re-fetch.

## Rollout

1. Deploy this PHP change (still using fake hash when secret is unset — zero
   regression risk).
2. Set `STREAMAX_STREAM_SECRET` in `config.php` on the PHP side.
3. Configure the same secret on the camera server's nginx/middleware.
4. Enable validation on the camera server.
5. Watch logs for any client that's caching URLs past 1 hour.
