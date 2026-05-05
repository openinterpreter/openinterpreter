use crate::error::TransportError;
use crate::request::Request;
use http::HeaderMap;
use http::header::RETRY_AFTER;
use rand::Rng;
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u64,
    pub base_delay: Duration,
    pub retry_on: RetryOn,
}

#[derive(Debug, Clone)]
pub struct RetryOn {
    pub retry_429: bool,
    pub retry_5xx: bool,
    pub retry_transport: bool,
}

impl RetryOn {
    pub fn should_retry(&self, err: &TransportError, attempt: u64, max_attempts: u64) -> bool {
        if attempt >= max_attempts {
            return false;
        }
        match err {
            TransportError::Http { status, .. } => {
                (self.retry_429 && status.as_u16() == 429)
                    || (self.retry_5xx && status.is_server_error())
            }
            TransportError::Timeout | TransportError::Network(_) => self.retry_transport,
            _ => false,
        }
    }
}

pub fn backoff(base: Duration, attempt: u64) -> Duration {
    if attempt == 0 {
        return base;
    }
    let exp = 2u64.saturating_pow(attempt as u32 - 1);
    let millis = base.as_millis() as u64;
    let raw = millis.saturating_mul(exp);
    let jitter: f64 = rand::rng().random_range(0.9..1.1);
    Duration::from_millis((raw as f64 * jitter) as u64)
}

fn retry_after_delay(headers: Option<&HeaderMap>) -> Option<Duration> {
    let headers = headers?;
    [
        RETRY_AFTER.as_str(),
        "x-retry-after",
        "msh-cooldown-seconds",
    ]
    .iter()
    .filter_map(|name| headers.get(*name))
    .filter_map(|value| value.to_str().ok())
    .filter_map(|value| value.trim().parse::<u64>().ok())
    .map(Duration::from_secs)
    .max()
}

fn retry_delay(base: Duration, attempt: u64, err: &TransportError) -> Duration {
    let backoff_delay = backoff(base, attempt);
    let retry_after = match err {
        TransportError::Http { headers, .. } => retry_after_delay(headers.as_ref()),
        TransportError::RetryLimit
        | TransportError::Timeout
        | TransportError::Network(_)
        | TransportError::Build(_) => None,
    };
    retry_after
        .filter(|delay| *delay > backoff_delay)
        .unwrap_or(backoff_delay)
}

pub async fn run_with_retry<T, F, Fut>(
    policy: RetryPolicy,
    mut make_req: impl FnMut() -> Request,
    op: F,
) -> Result<T, TransportError>
where
    F: Fn(Request, u64) -> Fut,
    Fut: Future<Output = Result<T, TransportError>>,
{
    for attempt in 0..=policy.max_attempts {
        let req = make_req();
        match op(req, attempt).await {
            Ok(resp) => return Ok(resp),
            Err(err)
                if policy
                    .retry_on
                    .should_retry(&err, attempt, policy.max_attempts) =>
            {
                sleep(retry_delay(policy.base_delay, attempt + 1, &err)).await;
            }
            Err(err) => return Err(err),
        }
    }
    Err(TransportError::RetryLimit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;
    use http::HeaderValue;
    use http::StatusCode;

    #[test]
    fn retry_delay_honors_retry_after_header() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("10"));
        let err = TransportError::Http {
            status: StatusCode::TOO_MANY_REQUESTS,
            url: None,
            headers: Some(headers),
            body: None,
        };

        assert_eq!(
            retry_delay(Duration::from_millis(200), 1, &err),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn retry_delay_honors_provider_cooldown_header() {
        let mut headers = HeaderMap::new();
        headers.insert("msh-cooldown-seconds", HeaderValue::from_static("10"));
        let err = TransportError::Http {
            status: StatusCode::TOO_MANY_REQUESTS,
            url: None,
            headers: Some(headers),
            body: None,
        };

        assert_eq!(
            retry_delay(Duration::from_millis(200), 1, &err),
            Duration::from_secs(10)
        );
    }
}
