use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpmlSource {
    pub title: String,
    pub xml_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename = "opml")]
struct Opml {
    #[serde(rename = "@version")]
    version: String,
    head: Head,
    body: Body,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct Head {
    title: String,
    #[serde(rename = "dateCreated", skip_serializing_if = "Option::is_none")]
    date_created: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct Body {
    #[serde(rename = "outline", default)]
    outlines: Vec<Outline>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct Outline {
    #[serde(rename = "@text", default)]
    text: String,
    #[serde(rename = "@type", skip_serializing_if = "Option::is_none")]
    ty: Option<String>,
    #[serde(rename = "@xmlUrl", skip_serializing_if = "Option::is_none")]
    xml_url: Option<String>,
    #[serde(rename = "outline", default)]
    outlines: Vec<Outline>,
}

pub fn export_opml(sources: &[OpmlSource]) -> anyhow::Result<String> {
    export_opml_with_date(sources, Utc::now().to_rfc2822())
}

fn export_opml_with_date(sources: &[OpmlSource], date_created: String) -> anyhow::Result<String> {
    let opml = Opml {
        version: "2.0".to_owned(),
        head: Head { title: "subscriptions in flowerss".to_owned(), date_created: Some(date_created) },
        body: Body {
            outlines: sources
                .iter()
                .map(|source| Outline {
                    text: source.title.clone(),
                    ty: Some("rss".to_owned()),
                    xml_url: Some(source.xml_url.clone()),
                    outlines: Vec::new(),
                })
                .collect(),
        },
    };
    let body = quick_xml::se::to_string(&opml)?;
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{body}"))
}

pub fn import_opml(xml: &str) -> anyhow::Result<Vec<OpmlSource>> {
    let opml: Opml = quick_xml::de::from_str(xml).map_err(|_| anyhow::anyhow!("parse opml file error"))?;
    let mut out = Vec::new();

    for outline in opml.body.outlines {
        // Match Go GetFlattenOutlines exactly: include one level of children
        // with xmlUrl, then the top-level outline itself with xmlUrl. Do not
        // recurse deeper, even though recursive import would be more complete.
        for child in &outline.outlines {
            if let Some(xml_url) = non_empty(child.xml_url.as_deref()) {
                out.push(OpmlSource { title: child.text.clone(), xml_url: xml_url.to_owned() });
            }
        }
        if let Some(xml_url) = non_empty(outline.xml_url.as_deref()) {
            out.push(OpmlSource { title: outline.text, xml_url: xml_url.to_owned() });
        }
    }

    Ok(out)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_go_compatible_opml_shape() {
        let xml = export_opml_with_date(
            &[OpmlSource { title: "Example".to_owned(), xml_url: "https://example.com/feed".to_owned() }],
            "Mon, 02 Jan 2006 15:04:05 GMT".to_owned(),
        )
        .unwrap();

        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<title>subscriptions in flowerss</title>"));
        assert!(xml.contains("<dateCreated>Mon, 02 Jan 2006 15:04:05 GMT</dateCreated>"));
        assert!(xml.contains("text=\"Example\""));
        assert!(xml.contains("type=\"rss\""));
        assert!(xml.contains("xmlUrl=\"https://example.com/feed\""));
    }

    #[test]
    fn import_flattens_only_two_levels_like_go() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0"><head><title>subscriptions in flowerss</title></head><body>
  <outline text="Group">
    <outline text="Child" type="rss" xmlUrl="https://example.com/child.xml">
      <outline text="Grandchild" type="rss" xmlUrl="https://example.com/grandchild.xml"/>
    </outline>
  </outline>
  <outline text="Top" type="rss" xmlUrl="https://example.com/top.xml"/>
</body></opml>"#;

        let imported = import_opml(xml).unwrap();
        assert_eq!(
            imported,
            vec![
                OpmlSource { title: "Child".to_owned(), xml_url: "https://example.com/child.xml".to_owned() },
                OpmlSource { title: "Top".to_owned(), xml_url: "https://example.com/top.xml".to_owned() },
            ]
        );
    }

    #[test]
    fn import_error_message_matches_go_wrapper() {
        let err = import_opml("not xml").unwrap_err();
        assert_eq!(err.to_string(), "parse opml file error");
    }
}
