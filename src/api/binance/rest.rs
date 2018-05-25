use super::*;
use std::fmt;
use openssl::pkey::{PKey, Private};
use openssl::sign::Signer;
use openssl::hash::MessageDigest;
use hex;
use hyper::{Method, Request, Body};
use std::time::{SystemTime, UNIX_EPOCH};

struct QueryString {
    query: String,
}

impl QueryString {
    fn new() -> Self {
        QueryString {
            query: String::new(),
        }
    }

    fn push<P: fmt::Display>(&mut self, name: &str, arg: P) {
        if !self.query.is_empty() {
            self.query += "&";
        }
        self.query += &format!("{}={}", name, arg);
    }

    fn into_string(self) -> String {
        self.query
    }

    fn into_string_with_signature(mut self, key: &PKey<Private>) -> String {
        let mut signer = Signer::new(MessageDigest::sha256(), key).unwrap();
        signer.update(self.query.as_bytes()).unwrap();
        let signature = hex::encode(&signer.sign_to_vec().unwrap());
        self.push("signature", &signature);
        self.query
    }
}

trait AsStr {
    fn as_str(&self) -> &'static str;
}

impl AsStr for Side {
    fn as_str(&self) -> &'static str {
        match self {
            Side::Ask => "SELL",
            Side::Bid => "BUY",
        }
    }
}

impl AsStr for TimeInForce {
    fn as_str(&self) -> &'static str {
        match self {
            TimeInForce::GoodTilCanceled => "GTC",
            TimeInForce::FillOrKilll => "FOK",
            TimeInForce::ImmediateOrCancel => "IOC",
        }
    }
}

impl Client {
    fn order(&self, order: Order) {
        let mut query = QueryString::new();
        query.push("symbol", self.params.symbol.to_uppercase());
        query.push("side", order.side.as_str());
        query.push("type", "LIMIT");
        query.push("timeInForce", order.time_in_force.as_str());
        query.push("quantity", &order.size);
        query.push("price", &order.price);
        if let Some(order_id) = &order.order_id {
            query.push("newClientOrderId", order_id);
        }
        query.push("recvWindow", order.time_window);
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        query.push("timestamp", timestamp.as_secs() + timestamp.subsec_millis() as u64);


        let address = format!(
            "{}/api/v3/order?{}",
            self.params.http_address,
            &query.into_string_with_signature(&self.secret_key)
        );

        let request = Request::builder()
            .method(Method::POST)
            .uri(&address)
            .header("X-MBX-APIKEY", self.params.api_key.as_bytes())
            .body(Body::empty())
            .unwrap();
        
        let https = hyper_tls::HttpsConnector::new(2).unwrap();
        let client = hyper::Client::builder().build::<_, hyper::Body>(https);
        let fut = client.request(request).and_then(|res| {
            Ok(())
        });
    }
}
