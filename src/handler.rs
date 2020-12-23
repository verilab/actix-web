use std::future::{ready, Future, Ready};
use std::marker::PhantomData;
use std::task::{Context, Poll};

use actix_http::Error;
use actix_service::{Service, ServiceFactory};

use crate::extract::FromRequest;
use crate::responder::Responder;
use crate::service::{ServiceRequest, ServiceResponse};

/// Async handler converter factory
pub trait Factory<T, R, O>: Clone + 'static
where
    R: Future<Output = O>,
    O: Responder,
{
    fn call(&self, param: T) -> R;
}

impl<F, R, O> Factory<(), R, O> for F
where
    F: Fn() -> R + Clone + 'static,
    R: Future<Output = O>,
    O: Responder,
{
    fn call(&self, _: ()) -> R {
        (self)()
    }
}

#[doc(hidden)]
/// Extract arguments from request, run factory function and make response.
pub struct Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
    R: Future<Output = O>,
    T: FromRequest,
    O: Responder,
{
    hnd: F,
    _t: PhantomData<(T, R, O)>,
}

impl<F, T, R, O> Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
    R: Future<Output = O>,
    T: FromRequest,
    O: Responder,
{
    pub fn new(hnd: F) -> Self {
        Handler {
            hnd,
            _t: PhantomData,
        }
    }
}

impl<F, T, R, O> Clone for Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
    R: Future<Output = O>,
    T: FromRequest,
    O: Responder,
{
    fn clone(&self) -> Self {
        Handler {
            hnd: self.hnd.clone(),
            _t: PhantomData,
        }
    }
}

impl<F, T, R, O> ServiceFactory for Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    T: FromRequest,
    R: Future<Output = O>,
    T: FromRequest,
    O: Responder,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = Self;
    type InitError = ();
    type Future = Ready<Result<Self::Service, ()>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ready(Ok(self.clone()))
    }
}

// Handler is both the Service and ServiceFactory type.
impl<F, T, R, O> Service for Handler<F, T, R, O>
where
    F: Factory<T, R, O>,
    R: Future<Output = O>,
    T: FromRequest,
    O: Responder,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&self, req: Self::Request) -> Self::Future {
        let (req, mut payload) = req.into_parts();
        let handle = self.hnd.clone();
        async move {
            // extract items from request.
            match T::from_request(&req, &mut payload).await {
                Ok(item) => {
                    // pass item to handler function and call Responder on the return type.
                    let res = handle
                        .call(item)
                        .await
                        .respond_to(&req)
                        .await
                        .unwrap_or_else(|e| e.into().into());

                    // always return a ServiceResponse type and Error type is a placeholder type
                    // that never used.
                    Ok(ServiceResponse::new(req, res))
                }
                Err(e) => {
                    let req = ServiceRequest::new(req);
                    Ok(req.error_response(e))
                }
            }
        }
    }
}

/// FromRequest trait impl for tuples
macro_rules! factory_tuple ({ $(($n:tt, $T:ident)),+} => {
    impl<Func, $($T,)+ Res, O> Factory<($($T,)+), Res, O> for Func
    where Func: Fn($($T,)+) -> Res + Clone + 'static,
          Res: Future<Output = O>,
          O: Responder,
    {
        fn call(&self, param: ($($T,)+)) -> Res {
            (self)($(param.$n,)+)
        }
    }
});

#[rustfmt::skip]
mod m {
    use super::*;

factory_tuple!((0, A));
factory_tuple!((0, A), (1, B));
factory_tuple!((0, A), (1, B), (2, C));
factory_tuple!((0, A), (1, B), (2, C), (3, D));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
factory_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
}
