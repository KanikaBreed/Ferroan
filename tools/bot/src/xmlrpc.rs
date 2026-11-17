// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-net/src/xmlrpc.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69

//! XML-RPC values as LLSD, for `login_to_simulator`.

use base64::Engine as _;
use ferroan_llsd::xml::{self, Element};
use ferroan_llsd::{Llsd, Map};

fn write_value(value: &Llsd, out: &mut String) {
    out.push_str("<value>");
    match value {
        Llsd::Undef => out.push_str("<string></string>"),
        Llsd::Boolean(flag) => {
            out.push_str("<boolean>");
            out.push_str(if *flag { "1" } else { "0" });
            out.push_str("</boolean>");
        }
        Llsd::Integer(integer) => {
            out.push_str("<int>");
            out.push_str(&integer.to_string());
            out.push_str("</int>");
        }
        Llsd::Real(real) => {
            out.push_str("<double>");
            out.push_str(&real.to_string());
            out.push_str("</double>");
        }
        Llsd::String(text) | Llsd::Uri(text) => {
            out.push_str("<string>");
            xml::escape(text, out);
            out.push_str("</string>");
        }
        Llsd::Uuid(id) => {
            out.push_str("<string>");
            out.push_str(&id.to_string());
            out.push_str("</string>");
        }
        Llsd::Date(ticks) => {
            out.push_str("<double>");
            out.push_str(&ticks.to_string());
            out.push_str("</double>");
        }
        Llsd::Binary(bytes) => {
            out.push_str("<base64>");
            out.push_str(&base64::engine::general_purpose::STANDARD.encode(bytes));
            out.push_str("</base64>");
        }
        Llsd::Array(items) => {
            out.push_str("<array><data>");
            for item in items {
                write_value(item, out);
            }
            out.push_str("</data></array>");
        }
        Llsd::Map(map) => {
            out.push_str("<struct>");
            for (key, item) in map {
                out.push_str("<member><name>");
                xml::escape(key, out);
                out.push_str("</name>");
                write_value(item, out);
                out.push_str("</member>");
            }
            out.push_str("</struct>");
        }
    }
    out.push_str("</value>");
}

/// A `methodCall` with one parameter.
pub fn build_call(method: &str, param: &Llsd) -> String {
    let mut body = String::from("<?xml version=\"1.0\"?><methodCall><methodName>");
    xml::escape(method, &mut body);
    body.push_str("</methodName><params><param>");
    write_value(param, &mut body);
    body.push_str("</param></params></methodCall>");
    body
}

fn parse_value(element: &Element) -> Llsd {
    // `<value>` holds a typed child, or bare text that means a string.
    // Tranquillity's login reply uses `<value>true</value>`.
    let Some(child) = element.elements().next() else {
        return Llsd::String(element.text());
    };
    match child.name.as_str() {
        "string" => Llsd::String(child.text()),
        "int" | "i4" | "i8" => Llsd::Integer(child.text().trim().parse().unwrap_or(0)),
        "boolean" => Llsd::Boolean(child.text().trim() == "1" || child.text().trim() == "true"),
        "double" => Llsd::Real(child.text().trim().parse().unwrap_or(0.0)),
        "base64" => Llsd::Binary(
            base64::engine::general_purpose::STANDARD
                .decode(child.text().split_whitespace().collect::<String>())
                .unwrap_or_default(),
        ),
        "dateTime.iso8601" => Llsd::String(child.text()),
        "array" => {
            let mut items = Vec::new();
            if let Some(data) = child.child("data") {
                for value in data.elements().filter(|element| element.name == "value") {
                    items.push(parse_value(value));
                }
            }
            Llsd::Array(items)
        }
        "struct" => {
            let mut map = Map::new();
            for member in child.elements().filter(|element| element.name == "member") {
                let name = member
                    .child("name")
                    .map(|name| name.text())
                    .unwrap_or_default();
                let value = member.child("value").map(parse_value).unwrap_or_default();
                map.insert(name, value);
            }
            Llsd::Map(map)
        }
        "nil" => Llsd::Undef,
        _ => Llsd::String(child.text()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum XmlRpcError {
    #[error("xml parse error: {0}")]
    Xml(#[from] xml::XmlError),
    #[error("fault {code}: {message}")]
    Fault { code: i32, message: String },
    #[error("malformed response")]
    Malformed,
}

/// The first parameter of a `methodResponse`.
pub fn parse_response(body: &[u8]) -> Result<Llsd, XmlRpcError> {
    let root = xml::parse(body)?;
    if root.name != "methodResponse" {
        return Err(XmlRpcError::Malformed);
    }
    if let Some(fault) = root.child("fault") {
        let value = fault.child("value").map(parse_value).unwrap_or_default();
        return Err(XmlRpcError::Fault {
            code: value["faultCode"].as_i32(),
            message: value["faultString"].to_string_value(),
        });
    }
    let value = root
        .child("params")
        .and_then(|params| params.child("param"))
        .and_then(|param| param.child("value"))
        .ok_or(XmlRpcError::Malformed)?;
    Ok(parse_value(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_llsd::llsd_map;

    #[test]
    fn roundtrip() {
        let param = llsd_map! {
            "first" => "Test",
            "n" => 3,
            "b" => true,
        };
        let mut with_array = param;
        with_array.insert("opts", Llsd::Array(vec![Llsd::from("a")]));
        let document = build_call("login_to_simulator", &with_array);
        let response = document
            .replace("methodCall", "methodResponse")
            .replace("<methodName>login_to_simulator</methodName>", "");
        let value = parse_response(response.as_bytes()).unwrap();
        assert_eq!(value["first"].as_str(), "Test");
        assert_eq!(value["n"].as_i32(), 3);
        assert_eq!(value["opts"][0].as_str(), "a");
        assert!(value["b"].as_bool());
    }

    #[test]
    fn bare_string_values() {
        let response = b"<?xml version='1.0'?><methodResponse><params><param><value><struct><member><name>login</name><value>true</value></member></struct></value></param></params></methodResponse>";
        let value = parse_response(response).unwrap();
        assert_eq!(value["login"].as_str(), "true");
    }

    #[test]
    fn fault() {
        let response = b"<methodResponse><fault><value><struct><member><name>faultCode</name><value><int>4</int></value></member><member><name>faultString</name><value><string>Too many</string></value></member></struct></value></fault></methodResponse>";
        assert!(matches!(
            parse_response(response),
            Err(XmlRpcError::Fault { code: 4, .. })
        ));
    }
}
