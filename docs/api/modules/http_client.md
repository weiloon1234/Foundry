# http_client

Outbound HTTP: pooled transport, timeouts, safe retries, typed responses, and fakes

[Back to index](../index.md)

## foundry::http_client

```rust
pub type HttpClientResult<T> = Result<T, HttpClientError>;
enum HttpClientError { InvalidUrl, InvalidHeader, Build, Encode, Transport, Timeout, ConcurrencyClosed, Decode, Status, FakeExhausted }
  fn kind(&self) -> HttpClientErrorKind
  fn transport(message: impl Into<String>) -> Self
  fn status(&self) -> Option<StatusCode>
  fn timeout_duration(&self) -> Option<Duration>
enum HttpClientErrorKind { InvalidUrl, InvalidHeader, Build, Encode, Transport, Timeout, ConcurrencyClosed, Decode, Status, FakeExhausted }
struct HttpClient
  fn new() -> HttpClientResult<Self>
  fn builder() -> HttpClientBuilder
  fn from_transport<T>(transport: T) -> HttpClientResult<Self>
  fn request( &self, method: Method, target: impl Into<String>, ) -> HttpRequestBuilder
  fn get(&self, target: impl Into<String>) -> HttpRequestBuilder
  fn head(&self, target: impl Into<String>) -> HttpRequestBuilder
  fn post(&self, target: impl Into<String>) -> HttpRequestBuilder
  fn put(&self, target: impl Into<String>) -> HttpRequestBuilder
  fn patch(&self, target: impl Into<String>) -> HttpRequestBuilder
  fn delete(&self, target: impl Into<String>) -> HttpRequestBuilder
  async fn send(&self, request: HttpRequest) -> HttpClientResult<HttpResponse>
  async fn send_with_retry( &self, request: HttpRequest, retry_policy: RetryPolicy, ) -> HttpClientResult<HttpResponse>
  fn raw(&self) -> Option<&Client>
  fn base_url(&self) -> Option<&Url>
  fn default_headers(&self) -> &HeaderMap
  fn connect_timeout(&self) -> Option<Duration>
  fn request_timeout(&self) -> Option<Duration>
  fn max_concurrency(&self) -> usize
  fn retry_policy(&self) -> &RetryPolicy
struct HttpClientBuilder
  fn new() -> Self
  fn base_url(self, base_url: impl AsRef<str>) -> HttpClientResult<Self>
  fn default_header( self, name: impl AsRef<str>, value: impl AsRef<str>, ) -> HttpClientResult<Self>
  fn default_headers(self, headers: HeaderMap) -> Self
  fn connect_timeout(self, timeout: Option<Duration>) -> Self
  fn request_timeout(self, timeout: Option<Duration>) -> Self
  fn max_concurrency(self, max_concurrency: usize) -> Self
  fn retry_policy(self, retry_policy: RetryPolicy) -> Self
  fn transport<T>(self, transport: T) -> Self
  fn shared_transport(self, transport: Arc<dyn HttpTransport>) -> Self
  fn build(self) -> HttpClientResult<HttpClient>
struct HttpHeaderMap
  fn new() -> HeaderMap
  fn with_capacity(capacity: usize) -> HeaderMap<T>
  fn try_with_capacity( capacity: usize, ) -> Result<HeaderMap<T>, MaxSizeReached>
  fn len(&self) -> usize
  fn keys_len(&self) -> usize
  fn is_empty(&self) -> bool
  fn clear(&mut self)
  fn capacity(&self) -> usize
  fn reserve(&mut self, additional: usize)
  fn try_reserve(&mut self, additional: usize) -> Result<(), MaxSizeReached>
  fn get<K>(&self, key: K) -> Option<&T>
  fn get_mut<K>(&mut self, key: K) -> Option<&mut T>
  fn get_all<K>(&self, key: K) -> GetAll<'_, T>
  fn contains_key<K>(&self, key: K) -> bool
  fn iter(&self) -> Iter<'_, T>
  fn iter_mut(&mut self) -> IterMut<'_, T>
  fn keys(&self) -> Keys<'_, T>
  fn values(&self) -> Values<'_, T>
  fn values_mut(&mut self) -> ValuesMut<'_, T>
  fn drain(&mut self) -> Drain<'_, T>
  fn entry<K>(&mut self, key: K) -> Entry<'_, T>
  fn try_entry<K>( &mut self, key: K, ) -> Result<Entry<'_, T>, InvalidHeaderName>
  fn insert<K>(&mut self, key: K, val: T) -> Option<T>
  fn try_insert<K>( &mut self, key: K, val: T, ) -> Result<Option<T>, MaxSizeReached>
  fn append<K>(&mut self, key: K, value: T) -> bool
  fn try_append<K>( &mut self, key: K, value: T, ) -> Result<bool, MaxSizeReached>
  fn remove<K>(&mut self, key: K) -> Option<T>
struct HttpHeaderName
  fn from_bytes(src: &[u8]) -> Result<HeaderName, InvalidHeaderName>
  fn from_lowercase(src: &[u8]) -> Result<HeaderName, InvalidHeaderName>
  const fn from_static(src: &'static str) -> HeaderName
  fn as_str(&self) -> &str
struct HttpHeaderValue
  const fn from_static(src: &'static str) -> HeaderValue
  fn from_str(src: &str) -> Result<HeaderValue, InvalidHeaderValue>
  fn from_name(name: HeaderName) -> HeaderValue
  fn from_bytes(src: &[u8]) -> Result<HeaderValue, InvalidHeaderValue>
  fn from_maybe_shared<T>(src: T) -> Result<HeaderValue, InvalidHeaderValue>
  unsafe fn from_maybe_shared_unchecked<T>(src: T) -> HeaderValue
  fn to_str(&self) -> Result<&str, ToStrError>
  fn len(&self) -> usize
  fn is_empty(&self) -> bool
  fn as_bytes(&self) -> &[u8] ⓘ
  fn set_sensitive(&mut self, val: bool)
  fn is_sensitive(&self) -> bool
struct HttpMethod
  const GET: Method
  const POST: Method
  const PUT: Method
  const DELETE: Method
  const HEAD: Method
  const OPTIONS: Method
  const CONNECT: Method
  const PATCH: Method
  const TRACE: Method
  fn from_bytes(src: &[u8]) -> Result<Method, InvalidMethod>
  fn is_safe(&self) -> bool
  fn is_idempotent(&self) -> bool
  fn as_str(&self) -> &str
struct HttpRequest
  fn new(method: Method, url: Url) -> Self
  fn with_headers(self, headers: HeaderMap) -> Self
  fn with_header(self, name: HeaderName, value: HeaderValue) -> Self
  fn with_body(self, body: impl Into<Vec<u8>>) -> Self
  fn method(&self) -> &Method
  fn url(&self) -> &Url
  fn redacted_url(&self) -> String
  fn headers(&self) -> &HeaderMap
  fn header(&self, name: &str) -> Option<&str>
  fn body(&self) -> Option<&[u8]>
  fn json_body<T>(&self) -> HttpClientResult<T>
  fn query_pairs(&self) -> Vec<(String, String)>
struct HttpRequestBuilder
  fn header(self, name: impl AsRef<str>, value: impl AsRef<str>) -> Self
  fn bearer_auth(self, token: impl AsRef<str>) -> Self
  fn query<T>(self, query: &T) -> Self
  fn query_pair(self, key: impl Into<String>, value: impl ToString) -> Self
  fn json<T>(self, value: &T) -> Self
  fn body(self, body: impl Into<Vec<u8>>) -> Self
  fn retry_policy(self, policy: RetryPolicy) -> Self
  fn timeout(self, timeout: Option<Duration>) -> Self
  fn build(self) -> HttpClientResult<HttpRequest>
  async fn send(self) -> HttpClientResult<HttpResponse>
struct HttpResponse
  fn new(status: StatusCode) -> Self
  fn from_json<T>(status: StatusCode, value: &T) -> HttpClientResult<Self>
  fn with_body(self, body: impl Into<Vec<u8>>) -> Self
  fn with_headers(self, headers: HeaderMap) -> Self
  fn with_header(self, name: HeaderName, value: HeaderValue) -> Self
  fn status(&self) -> StatusCode
  fn is_success(&self) -> bool
  fn headers(&self) -> &HeaderMap
  fn header(&self, name: &str) -> Option<&str>
  fn bytes(&self) -> &[u8] ⓘ
  fn into_bytes(self) -> Vec<u8> ⓘ
  fn text(&self) -> HttpClientResult<&str>
  fn json<T>(&self) -> HttpClientResult<T>
  fn ensure_success(&self) -> HttpClientResult<()>
  fn error_for_status(self) -> HttpClientResult<Self>
struct HttpStatus
  const fn from_u16(src: u16) -> Result<StatusCode, InvalidStatusCode>
  fn from_bytes(src: &[u8]) -> Result<StatusCode, InvalidStatusCode>
  const fn as_u16(&self) -> u16
  fn as_str(&self) -> &str
  fn canonical_reason(&self) -> Option<&'static str>
  fn is_informational(&self) -> bool
  fn is_success(&self) -> bool
  fn is_redirection(&self) -> bool
  fn is_client_error(&self) -> bool
  fn is_server_error(&self) -> bool
  const CONTINUE: StatusCode
  const SWITCHING_PROTOCOLS: StatusCode
  const PROCESSING: StatusCode
  const EARLY_HINTS: StatusCode
  const OK: StatusCode
  const CREATED: StatusCode
  const ACCEPTED: StatusCode
  const NON_AUTHORITATIVE_INFORMATION: StatusCode
  const NO_CONTENT: StatusCode
  const RESET_CONTENT: StatusCode
  const PARTIAL_CONTENT: StatusCode
  const MULTI_STATUS: StatusCode
  const ALREADY_REPORTED: StatusCode
  const IM_USED: StatusCode
  const MULTIPLE_CHOICES: StatusCode
  const MOVED_PERMANENTLY: StatusCode
  const FOUND: StatusCode
  const SEE_OTHER: StatusCode
  const NOT_MODIFIED: StatusCode
  const USE_PROXY: StatusCode
  const TEMPORARY_REDIRECT: StatusCode
  const PERMANENT_REDIRECT: StatusCode
  const BAD_REQUEST: StatusCode
  const UNAUTHORIZED: StatusCode
  const PAYMENT_REQUIRED: StatusCode
  const FORBIDDEN: StatusCode
  const NOT_FOUND: StatusCode
  const METHOD_NOT_ALLOWED: StatusCode
  const NOT_ACCEPTABLE: StatusCode
  const PROXY_AUTHENTICATION_REQUIRED: StatusCode
  const REQUEST_TIMEOUT: StatusCode
  const CONFLICT: StatusCode
  const GONE: StatusCode
  const LENGTH_REQUIRED: StatusCode
  const PRECONDITION_FAILED: StatusCode
  const PAYLOAD_TOO_LARGE: StatusCode
  const URI_TOO_LONG: StatusCode
  const UNSUPPORTED_MEDIA_TYPE: StatusCode
  const RANGE_NOT_SATISFIABLE: StatusCode
  const EXPECTATION_FAILED: StatusCode
  const IM_A_TEAPOT: StatusCode
  const MISDIRECTED_REQUEST: StatusCode
  const UNPROCESSABLE_ENTITY: StatusCode
  const LOCKED: StatusCode
  const FAILED_DEPENDENCY: StatusCode
  const TOO_EARLY: StatusCode
  const UPGRADE_REQUIRED: StatusCode
  const PRECONDITION_REQUIRED: StatusCode
  const TOO_MANY_REQUESTS: StatusCode
  const REQUEST_HEADER_FIELDS_TOO_LARGE: StatusCode
  const UNAVAILABLE_FOR_LEGAL_REASONS: StatusCode
  const INTERNAL_SERVER_ERROR: StatusCode
  const NOT_IMPLEMENTED: StatusCode
  const BAD_GATEWAY: StatusCode
  const SERVICE_UNAVAILABLE: StatusCode
  const GATEWAY_TIMEOUT: StatusCode
  const HTTP_VERSION_NOT_SUPPORTED: StatusCode
  const VARIANT_ALSO_NEGOTIATES: StatusCode
  const INSUFFICIENT_STORAGE: StatusCode
  const LOOP_DETECTED: StatusCode
  const NOT_EXTENDED: StatusCode
  const NETWORK_AUTHENTICATION_REQUIRED: StatusCode
struct HttpUrl
  fn parse(input: &str) -> Result<Url, ParseError>
  fn parse_with_params<I, K, V>( input: &str, iter: I, ) -> Result<Url, ParseError>
  fn join(&self, input: &str) -> Result<Url, ParseError>
  fn make_relative(&self, url: &Url) -> Option<String>
  fn options<'a>() -> ParseOptions<'a>
  fn as_str(&self) -> &str
  fn into_string(self) -> String
  fn origin(&self) -> Origin
  fn scheme(&self) -> &str
  fn is_special(&self) -> bool
  fn has_authority(&self) -> bool
  fn authority(&self) -> &str
  fn cannot_be_a_base(&self) -> bool
  fn username(&self) -> &str
  fn password(&self) -> Option<&str>
  fn has_host(&self) -> bool
  fn host_str(&self) -> Option<&str>
  fn host(&self) -> Option<Host<&str>>
  fn domain(&self) -> Option<&str>
  fn port(&self) -> Option<u16>
  fn port_or_known_default(&self) -> Option<u16>
  fn socket_addrs( &self, default_port_number: impl Fn() -> Option<u16>, ) -> Result<Vec<SocketAddr>, Error>
  fn path(&self) -> &str
  fn path_segments(&self) -> Option<Split<'_, char>>
  fn query(&self) -> Option<&str>
  fn query_pairs(&self) -> Parse<'_>
  fn fragment(&self) -> Option<&str>
  fn set_fragment(&mut self, fragment: Option<&str>)
  fn set_query(&mut self, query: Option<&str>)
  fn query_pairs_mut(&mut self) -> Serializer<'_, UrlQuery<'_>>
  fn set_path(&mut self, path: &str)
  fn path_segments_mut(&mut self) -> Result<PathSegmentsMut<'_>, ()>
  fn set_port(&mut self, port: Option<u16>) -> Result<(), ()>
  fn set_host(&mut self, host: Option<&str>) -> Result<(), ParseError>
  fn set_ip_host(&mut self, address: IpAddr) -> Result<(), ()>
  fn set_password(&mut self, password: Option<&str>) -> Result<(), ()>
  fn set_username(&mut self, username: &str) -> Result<(), ()>
  fn set_scheme(&mut self, scheme: &str) -> Result<(), ()>
  fn from_file_path<P>(path: P) -> Result<Url, ()>
  fn from_directory_path<P>(path: P) -> Result<Url, ()>
  fn to_file_path(&self) -> Result<PathBuf, ()>
struct RawHttpClient
  fn new() -> Client
  fn builder() -> ClientBuilder
  fn get<U>(&self, url: U) -> RequestBuilder
  fn post<U>(&self, url: U) -> RequestBuilder
  fn put<U>(&self, url: U) -> RequestBuilder
  fn patch<U>(&self, url: U) -> RequestBuilder
  fn delete<U>(&self, url: U) -> RequestBuilder
  fn head<U>(&self, url: U) -> RequestBuilder
  fn request<U>(&self, method: Method, url: U) -> RequestBuilder
  fn execute( &self, request: Request, ) -> impl Future<Output = Result<Response, Error>>
struct ReqwestTransport
  fn new(client: Client) -> Self
  fn raw(&self) -> &Client
struct RetryPolicy
  fn idempotent() -> Self
  fn none() -> Self
  fn max_attempts(self, max_attempts: usize) -> Self
  fn backoff(self, initial: Duration, maximum: Duration) -> Self
  fn retry_method(self, method: Method) -> Self
  fn do_not_retry_method(self, method: &Method) -> Self
  fn retry_status(self, status: StatusCode) -> Self
  fn do_not_retry_status(self, status: &StatusCode) -> Self
  fn retry_transport_errors(self, retry: bool) -> Self
  fn attempts(&self) -> usize
  fn retries_method(&self, method: &Method) -> bool
  fn retries_status(&self, status: StatusCode) -> bool
trait HttpTransport
  fn send<'life0, 'async_trait>(
```

## Notes

- The default client has no base URL, a 10-second connect timeout, a 30-second per-attempt request timeout, concurrency 64, and conservative retries for read-only methods.
- Mutation methods are not retried unless explicitly selected. `raw()` bypasses Foundry retry, timeout, concurrency, tracing, and fake behavior.
- Framework traces redact URL credentials and query values and never record header values or bodies.
