use std::{future};
use std::sync::{Arc};
use std::task::{Context, Poll};
use futures_util::future::BoxFuture;
use http_body::Body;
use tokio::sync::Mutex;
use tower_service::Service;

/// Custom error type for CompositeService
pub enum CompositeError<E> {
    NoServices,
    AllFailed(E),
}

/// A composite tower service
#[derive(Clone)]
pub struct CompositeService<S, Request> {
    services: Arc<Mutex<Vec<S>>>,
    _phantom: std::marker::PhantomData<Request>,
}

impl<S, Request> CompositeService<S, Request>
where
    S: Service<Request>,
{
    pub fn new() -> Self {
        CompositeService {
            services: Arc::new(Mutex::new(Vec::new())),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Add service to stack
    pub async fn push (&mut self, service: S) {
        self.services.lock().await.push(service);
    }
}

impl<S, Request> Service<Request> for CompositeService<S, Request>
where
    S: Service<Request> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Send + 'static,
    Request: Send + 'static,
{
    type Response = S::Response;
    type Error = CompositeError<S::Error>;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        /*
        let mut services = self.services.lock().unwrap();
        if services.is_empty() {
            return Poll::Ready(Err(CompositeError::NoServices));
        }
        for mut service in services.iter_mut() {
            if service.poll_ready(cx).is_ready() {
                return Poll::Ready(Ok(()));
            }
        }
        Poll::Pending
        */

        // We can't use .await here, so we'll need to handle this differently
        // This is a simplification and might not be ideal for all use cases
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let services = self.services.clone();
        Box::pin(async move {
            let mut services = services.lock().await;
            if services.is_empty() {
                return Err(CompositeError::NoServices);
            }

            let mut last_error = None;
            for mut service in services.iter_mut() {
                match service.call(request).await {
                    Ok(response) => return Ok(response),
                    Err(e) => {
                        last_error = Some(e);
                        break;
                    }
                }
            }

            Err(CompositeError::AllFailed(last_error.expect("Should have at least one error")))
        })
    }
}