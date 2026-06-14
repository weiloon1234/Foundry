# kernel

5 runtime kernels: HTTP, CLI, Scheduler, Worker, WebSocket

[Back to index](../index.md)

## foundry::kernel::cli

```rust
struct CliKernel
  fn new(app: AppContext, registrars: Vec<CommandRegistrar>) -> Self
  fn build_registry(&self) -> Result<CommandRegistry>
  fn app(&self) -> &AppContext
  async fn run(self) -> Result<()>
  async fn run_with_args<I, T>(self, args: I) -> Result<()>
```

## foundry::kernel::http

```rust
struct BoundHttpServer
  fn local_addr(&self) -> SocketAddr
  async fn serve(self) -> Result<()>
struct HttpKernel
  fn new( app: AppContext, routes: Vec<RouteRegistrar>, middlewares: Vec<MiddlewareConfig>, observability: Option<ObservabilityOptions>, spa_dir: Option<PathBuf>, ) -> Self
  fn app(&self) -> &AppContext
  fn build_router(&self) -> Result<Router>
  async fn bind(self) -> Result<BoundHttpServer>
  async fn serve(self) -> Result<()>
```

## foundry::kernel::scheduler

```rust
struct SchedulerKernel
  fn new(app: AppContext, registry: ScheduleRegistry) -> Result<Self>
  fn app(&self) -> &AppContext
  async fn tick(&self) -> Result<Vec<ScheduleId>>
  async fn run_once(&self) -> Result<Vec<ScheduleId>>
  async fn run_once_at(&self, now: DateTime) -> Result<Vec<ScheduleId>>
  async fn tick_at(&self, now: DateTime) -> Result<Vec<ScheduleId>>
  async fn run(self) -> Result<()>
```

## foundry::kernel::websocket

```rust
struct BoundWebSocketServer
  fn local_addr(&self) -> SocketAddr
  async fn serve(self) -> Result<()>
struct WebSocketKernel
  fn new(app: AppContext) -> Self
  fn app(&self) -> &AppContext
  async fn bind(self) -> Result<BoundWebSocketServer>
  async fn serve(self) -> Result<()>
```

## foundry::kernel::worker

```rust
struct WorkerKernel
  fn new(app: AppContext) -> Result<Self>
  fn app(&self) -> &AppContext
  async fn run(self) -> Result<()>
  async fn run_once(&self) -> Result<bool>
```

