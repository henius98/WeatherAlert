pub mod domain;

#[cfg(target_arch = "wasm32")]
pub mod infra;

#[cfg(target_arch = "wasm32")]
mod entry {
  use crate::infra::dispatcher::Dispatcher;
  use worker::*;

  #[event(scheduled)]
  pub async fn scheduled(event: ScheduledEvent, env: Env, ctx: ScheduleContext) {
    Dispatcher::dispatch_scheduled(event, env, ctx).await;
  }

  #[event(fetch)]
  pub async fn fetch(req: Request, env: Env, ctx: Context) -> Result<Response> {
    Dispatcher::dispatch_fetch(req, env, ctx).await
  }
}
