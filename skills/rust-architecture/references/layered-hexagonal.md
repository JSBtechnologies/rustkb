---
id: architecture/layered-hexagonal
title: Layered and hexagonal architecture in Rust
summary: >-
  Ports and adapters the Rust way: concrete types by default, traits only at boundaries to
  external systems that need a second implementation, generics monomorphised at the composition
  root, dyn or enums when runtime choice is needed, and hand-written fakes for tests.
area: architecture
tags: [hexagonal, ports-and-adapters, clean-architecture, dependency-injection, traits, generics, dyn, testing, fakes]
rust: "1.96"
edition: "2024"
crates:
  async-trait: "0.1"
  thiserror: "2.0"
  tokio: "1.53"
  sqlx: "0.9"
  mockall: "0.15"
verified: 2026-09-29
sources:
  - https://alistair.cockburn.us/hexagonal-architecture/
  - https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits.html
  - https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility
---

# Layered and hexagonal architecture in Rust

Hexagonal architecture ("ports and adapters") is useful in Rust — the domain stays free of IO and
is trivially testable — but agents port it from Java/C# wholesale: an interface per class, a DI
container, `Arc<dyn Trait>` everywhere. Rust needs far less machinery.

## The default shape

### HEX-01: Concrete types by default; a trait only where a boundary needs two implementations

Default: write concrete structs and functions. Introduce a trait (a *port*) only when **both**
hold:

1. The code talks to something outside the process or outside your control: database, message
   broker, HTTP API, filesystem, clock, randomness, email.
2. You need a second implementation *now*: an in-memory fake for tests, or a second real backend.

Pure domain logic (pricing, validation, state machines) never needs a trait — it has no IO, so
test it directly.

❌ Common agent output:

```rust,ignore
trait OrderValidator { fn validate(&self, o: &Order) -> Result<(), Error>; }
struct OrderValidatorImpl;
trait PriceCalculator { … }  struct PriceCalculatorImpl;
struct OrderService { validator: Arc<dyn OrderValidator>, calc: Arc<dyn PriceCalculator>, … }
```

✅ `fn validate(order: &Order) -> Result<(), ValidationError>` and `fn price(order: &Order) -> Money`
as plain functions in the `orders` module; the only trait is the `OrderRepository` port.

### HEX-02: Four roles, not four layers of indirection

| Role | Contains | Depends on |
|---|---|---|
| Domain | Types, invariants, pure rules, port traits, domain errors | `std`, `serde`, `thiserror` only |
| Application service | Use-case orchestration: validate → call ports → return domain result | Domain |
| Adapters | Port implementations (`PgOrderRepository`), inbound adapters (HTTP handlers, CLI) | Domain + infra crates |
| Composition root | `main`: builds adapters, injects them, starts the server | Everything |

Small projects: modules in one crate. Larger ones: `app-domain` (domain + services),
`app-postgres` (adapter), `app-server` (inbound adapter + composition root). See
`workspace-layout.md` WS-06 for when to split.

## Defining ports

### HEX-03: Port traits are small, async-aware and `Send`

Default: one trait per external capability, with only the methods the domain uses. Use native
`async fn`-in-trait (stable since 1.75), but in the *trait declaration* spell the return type as
`impl Future<Output = …> + Send` so futures can be spawned on a multi-threaded runtime; implementers
still write plain `async fn`.

```rust
use std::future::Future;

use crate::{Order, OrderId, Sku};

/// Storage failure as seen by the domain (adapter details stay in `source()`).
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    /// Backend unreachable or timed out; safe to retry.
    #[error("storage unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Port: persistence for orders.
pub trait OrderRepository: Send + Sync + 'static {
    /// Insert a new order and return its assigned id.
    fn insert(
        &self,
        sku: &Sku,
        quantity: u32,
    ) -> impl Future<Output = Result<OrderId, RepoError>> + Send;

    /// Fetch one order.
    fn get(&self, id: OrderId) -> impl Future<Output = Result<Option<Order>, RepoError>> + Send;
}
```

Writing `async fn insert(..)` directly in a public trait triggers the `async_fn_in_trait` lint
because callers cannot require the future to be `Send`. The `-> impl Future + Send` form fixes
that for every implementation. Async trait mechanics: `idioms/async`, `idioms/traits-generics`.

Never make a "generic repository" (`trait Repository<T, Id>` with `find_all`, `save`, `delete`
for every entity). Ports are shaped by what the use case needs, not by CRUD.

### HEX-04: Errors cross ports as domain errors

The port returns a domain error enum (`RepoError`) whose variants are what the *domain* can act on
(retryable vs. not-found vs. conflict). Adapter-specific errors (`sqlx::Error`) are wrapped as the
`source`, never exposed as the port's error type. The inbound adapter (HTTP) then maps domain errors
to status codes in one place (`web-services.md` SVC-04).

## Injecting adapters

### HEX-05: Inject with generics; stop the generics at the composition root

Default: services are generic over their ports (static dispatch, zero cost, no `dyn`
restrictions). The binary picks the concrete adapter once, with a type alias, so HTTP handlers and
state stay non-generic.

```rust,ignore
// domain crate
pub struct OrderService<R> { repo: R }

impl<R: OrderRepository> OrderService<R> {
    pub fn new(repo: R) -> Self { Self { repo } }

    pub async fn place(&self, sku: Sku, quantity: u32) -> Result<OrderId, PlaceOrderError> {
        if !(1..=100).contains(&quantity) {
            return Err(PlaceOrderError::InvalidQuantity(quantity));
        }
        Ok(self.repo.insert(&sku, quantity).await?)
    }
}

// server crate (composition root)
pub(crate) type Orders = OrderService<PgOrderRepository>;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) orders: Arc<Orders>,   // handlers take State<AppState>: no generics leak into axum
}
```

Don't thread a type parameter through `AppState<R>`, `router<R>()` and every handler — it
multiplies compile errors and gains nothing when production has one adapter.

### HEX-06: Use `dyn` or an enum only when the choice is made at runtime

| Situation | Use |
|---|---|
| One production adapter, a fake in tests | Generics (HEX-05) |
| Closed set of adapters chosen by config (`storage = "postgres" \| "sqlite"`) | `enum` with a `match` per method |
| Open set / plugins / heterogeneous collection (`Vec<Box<dyn Notifier>>`) | `Arc<dyn Trait>` |
| Generic instantiations measurably bloat compile time | `dyn` at that boundary |

Enum dispatch — no trait at all, exhaustive and easy to follow:

```rust,ignore
pub enum Mailer {
    Smtp(SmtpMailer),
    Recording(RecordingMailer), // tests and local dev
}

impl Mailer {
    pub async fn send(&self, to: &str, body: &str) -> Result<(), MailError> {
        match self {
            Self::Smtp(m) => m.send(to, body).await,
            Self::Recording(m) => {
                m.send(to, body);
                Ok(())
            }
        }
    }
}
```

`dyn` with async methods: native `async fn` / `-> impl Future` trait methods are **not**
dyn-compatible. For `Arc<dyn Trait>` use the `async-trait` crate (boxes each future), or return
`Pin<Box<dyn Future<Output = T> + Send + '_>>` by hand:

```rust,ignore
#[async_trait::async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, msg: &str) -> Result<(), NotifyError>;
}

let notifiers: Vec<Arc<dyn Notifier>> = vec![Arc::new(Slack::new(url)), Arc::new(Email::new(smtp))];
```

### HEX-07: No DI containers, service locators or global registries

Rust has no need for runtime DI frameworks: `main` constructs values and passes them to
constructors. A `static REGISTRY: OnceLock<HashMap<TypeId, Box<dyn Any>>>` or a "ServiceLocator"
struct hides dependencies, defeats the borrow checker's help, and turns missing wiring into runtime
panics. If `main` gets long, extract a `fn build(settings: &Settings) -> anyhow::Result<App>`.

## Testing through ports

### HEX-08: Test with hand-written fakes; reach for mocking crates last

Default: an in-memory fake implementing the port — usually 15–30 lines, reusable across tests,
and it tests behaviour instead of call sequences.

```rust,ignore
#[derive(Default)]
struct InMemoryRepo(Mutex<HashMap<u64, Order>>);

impl OrderRepository for InMemoryRepo {
    async fn insert(&self, sku: &Sku, quantity: u32) -> Result<OrderId, RepoError> {
        let mut map = self.0.lock().expect("poisoned");
        let id = OrderId(map.len() as u64 + 1);
        map.insert(id.0, Order { id, sku: sku.clone(), quantity, status: OrderStatus::Placed });
        Ok(id)
    }

    async fn get(&self, id: OrderId) -> Result<Option<Order>, RepoError> {
        Ok(self.0.lock().expect("poisoned").get(&id.0).cloned())
    }
}

#[tokio::test]
async fn rejects_zero_quantity() {
    let svc = OrderService::new(InMemoryRepo::default());
    let err = svc.place(Sku::try_from("A-1".to_owned()).unwrap(), 0).await.unwrap_err();
    assert!(matches!(err, PlaceOrderError::InvalidQuantity(0)));
}
```

Use `mockall` only to assert on interactions that are the behaviour (e.g. "exactly one email is
sent"), or for large third-party traits. Adapters themselves get integration tests against the
real thing (a Postgres container, a local HTTP stub), not mocks. Share fakes across crates via a
`test-util` feature (MOD-09).

## Transactions and cross-port consistency

### HEX-09: Keep transactions inside one adapter method

Default: if a use case must write atomically, give the port one method that does the whole
operation (`place_order_and_reserve_stock`) and implement the transaction inside the Postgres
adapter. Generic "unit of work" abstractions over `sqlx::Transaction` fight lifetimes and leak
the driver into the domain.

Exception: when many use cases compose writes, pass an explicit transaction handle at the
*application service* level of the adapter crate — but keep the domain crate unaware of it.

## Anti-pattern summary

### HEX-10: Over-abstraction smells to remove on sight

- A trait with exactly one implementation and no test fake → delete the trait.
- `FooService` + `FooServiceImpl` + `IFoo` naming → one concrete `Foo`.
- `Arc<dyn Trait>` for every field when generics or concrete types would do.
- `Box<dyn Error>` as a domain error type → typed enum (`idioms/error-handling`).
- Domain types deriving `sqlx::FromRow` or implementing `IntoResponse` → map in the adapter.
- A `Repository<T>` god-trait with methods the domain never calls.
- DTO ↔ entity ↔ model copies of the same struct with no differing invariants — share the type
  until the representations actually diverge.

Related: `modules-and-boundaries.md` (dependency direction), `web-services.md` (inbound adapter),
`idioms/traits-generics` (static vs. dynamic dispatch trade-offs).
