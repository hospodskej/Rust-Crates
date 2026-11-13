//! `Cache-Control: no-store` for private pages.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use http::header::{CACHE_CONTROL, HeaderValue};
use http::{Request, Response};
use pin_project_lite::pin_project;
use tower_layer::Layer;
use tower_service::Service;

/// A layer that sends `Cache-Control: no-store`, so neither the browser nor
/// a proxy keeps a copy of the page. Put it on the admin, account and
/// checkout routes: after signing out, the Back button must not show them.
///
/// It replaces any `Cache-Control` the handler set.
///
/// ```
/// use axum::{Router, routing::get};
/// use cuticle::{Cuticle, NoStore};
///
/// let account: Router = Router::new()
///     .route("/account", get(|| async { "private" }))
///     .layer(NoStore);
///
/// let app: Router = Router::new()
///     .route("/", get(|| async { "public" }))
///     .merge(account)
///     .layer(Cuticle::new().into_layer());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoStore;

impl<S> Layer<S> for NoStore {
    type Service = NoStoreService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        NoStoreService { inner }
    }
}

/// The [`Service`] made by [`NoStore`].
#[derive(Clone, Copy, Debug)]
pub struct NoStoreService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for NoStoreService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = NoStoreFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        NoStoreFuture {
            inner: self.inner.call(req),
        }
    }
}

pin_project! {
    /// The response future of [`NoStoreService`].
    #[derive(Debug)]
    pub struct NoStoreFuture<F> {
        #[pin]
        inner: F,
    }
}

impl<F, ResBody, E> Future for NoStoreFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut response = ready!(self.project().inner.poll(cx))?;
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        Poll::Ready(Ok(response))
    }
}
