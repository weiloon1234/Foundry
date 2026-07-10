# public

[Back to index](../index.md)

## foundry::public

```rust
derive ApiSchema
derive AppEnum
derive FoundryId
derive Model
derive Projection
derive TS
derive Validate
struct Cookie
  fn new<N, V>(name: N, value: V) -> Cookie<'c>
  fn named<N>(name: N) -> Cookie<'c>
  fn build<C>(base: C) -> CookieBuilder<'c>
  fn parse<S>(s: S) -> Result<Cookie<'c>, ParseError>
  fn parse_encoded<S>(s: S) -> Result<Cookie<'c>, ParseError>
  fn split_parse<S>(string: S) -> SplitCookies<'c>
  fn split_parse_encoded<S>(string: S) -> SplitCookies<'c>
  fn into_owned(self) -> Cookie<'static>
  fn name(&self) -> &str
  fn value(&self) -> &str
  fn value_trimmed(&self) -> &str
  fn name_value(&self) -> (&str, &str)
  fn name_value_trimmed(&self) -> (&str, &str)
  fn http_only(&self) -> Option<bool>
  fn secure(&self) -> Option<bool>
  fn same_site(&self) -> Option<SameSite>
  fn partitioned(&self) -> Option<bool>
  fn max_age(&self) -> Option<Duration>
  fn path(&self) -> Option<&str>
  fn domain(&self) -> Option<&str>
  fn expires(&self) -> Option<Expiration>
  fn expires_datetime(&self) -> Option<OffsetDateTime>
  fn set_name<N>(&mut self, name: N)
  fn set_value<V>(&mut self, value: V)
  fn set_http_only<T>(&mut self, value: T)
  fn set_secure<T>(&mut self, value: T)
  fn set_same_site<T>(&mut self, value: T)
  fn set_partitioned<T>(&mut self, value: T)
  fn set_max_age<D>(&mut self, value: D)
  fn set_path<P>(&mut self, path: P)
  fn unset_path(&mut self)
  fn set_domain<D>(&mut self, domain: D)
  fn unset_domain(&mut self)
  fn set_expires<T>(&mut self, time: T)
  fn unset_expires(&mut self)
  fn make_permanent(&mut self)
  fn make_removal(&mut self)
  fn name_raw(&self) -> Option<&'c str>
  fn value_raw(&self) -> Option<&'c str>
  fn path_raw(&self) -> Option<&'c str>
  fn domain_raw(&self) -> Option<&'c str>
  fn encoded<'a>(&'a self) -> Display<'a, 'c>
  fn stripped<'a>(&'a self) -> Display<'a, 'c>
struct CookieJar
  fn from_headers(headers: &HeaderMap) -> CookieJar
  fn new() -> CookieJar
  fn get(&self, name: &str) -> Option<&Cookie<'static>>
  fn remove<C>(self, cookie: C) -> CookieJar
  fn add<C>(self, cookie: C) -> CookieJar
  fn iter(&self) -> impl Iterator<Item = &Cookie<'static>>
```
