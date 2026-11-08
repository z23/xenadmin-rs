use super::*;
use roxmltree::{Document, Node};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredNfsExport {
    pub path: String,
    pub access: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredIscsiTarget {
    pub portal: String,
    pub port: u16,
    pub iqn: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredIscsiLun {
    pub scsi_id: String,
    pub lun: Option<u64>,
    pub size_bytes: Option<u64>,
    pub vendor: String,
    pub serial: String,
}
fn invalid() -> Error {
    Error::Protocol("invalid storage discovery XML")
}
fn document<'a>(xml: &'a str, root: &str) -> Result<Document<'a>, Error> {
    if xml.len() > 512 * 1024 {
        return Err(Error::Protocol("storage discovery exceeds size limit"));
    }
    let doc = Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 16000,
        },
    )
    .map_err(|_| invalid())?;
    if doc.root_element().tag_name().name() != root
        || doc.descendants().any(|n| n.ancestors().count() > 8)
    {
        return Err(invalid());
    }
    Ok(doc)
}
fn value<'a>(node: Node<'a, '_>, field: &str) -> Result<Option<&'a str>, Error> {
    let mut fields = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(field));
    let result = fields.next();
    if fields.next().is_some() {
        return Err(invalid());
    }
    result
        .map(|n| {
            if n.children().any(|n| n.is_element())
                || n.children().filter(|n| n.is_text()).count() > 1
            {
                return Err(invalid());
            }
            Ok(n.children()
                .find(|n| n.is_text())
                .and_then(|n| n.text())
                .unwrap_or("")
                .trim())
        })
        .transpose()
}
fn required<'a>(node: Node<'a, '_>, field: &str) -> Result<&'a str, Error> {
    value(node, field)?
        .filter(|v| !v.is_empty())
        .ok_or_else(invalid)
}
fn number(node: Node<'_, '_>, field: &str) -> Result<Option<u64>, Error> {
    value(node, field)?
        .filter(|v| !v.is_empty())
        .map(|v| v.parse().map_err(|_| invalid()))
        .transpose()
}
fn rows<'a>(doc: &'a Document<'a>, kind: &str) -> Result<Vec<Node<'a, 'a>>, Error> {
    let rows = doc
        .root_element()
        .children()
        .filter(|n| n.is_element())
        .collect::<Vec<_>>();
    if rows.len() > 1000 || rows.iter().any(|n| n.tag_name().name() != kind) {
        return Err(invalid());
    }
    Ok(rows)
}
pub(super) fn exports(xml: &str) -> Result<Vec<DiscoveredNfsExport>, Error> {
    let doc = document(xml, "nfs-exports")?;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for row in rows(&doc, "Export")? {
        let p = required(row, "Path")?;
        // The driver's NFSv4 pseudofs enumeration returns immediate relative children.
        let path = if p.starts_with('/') {
            p.to_owned()
        } else if !p.contains('/') && p != "." && p != ".." {
            format!("/{p}")
        } else {
            return Err(invalid());
        };
        if path.len() > 4096 || path.chars().any(char::is_control) || !seen.insert(path.clone()) {
            return Err(invalid());
        }
        result.push(DiscoveredNfsExport {
            path,
            access: clean(value(row, "Accesslist")?.unwrap_or("Unknown"), 512),
        });
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}
pub(super) fn targets(xml: &str) -> Result<Vec<DiscoveredIscsiTarget>, Error> {
    let doc = document(xml, "iscsi-target-iqns")?;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for row in rows(&doc, "TGT")? {
        let iqn = required(row, "TargetIQN")?;
        if iqn == "*" {
            continue;
        }
        let portal = required(row, "IPAddress")?;
        let port = u16::try_from(number(row, "Port")?.unwrap_or(3260)).map_err(|_| invalid())?;
        if port == 0
            || !crate::storage::storage_server(portal)
            || !crate::storage::storage_token(iqn)
            || !iqn.starts_with("iqn.")
        {
            return Err(invalid());
        }
        if seen.insert((portal.to_owned(), port, iqn.to_owned())) {
            result.push(DiscoveredIscsiTarget {
                portal: portal.into(),
                port,
                iqn: iqn.into(),
            });
        }
    }
    result.sort_by(|a, b| (&a.iqn, &a.portal, a.port).cmp(&(&b.iqn, &b.portal, b.port)));
    Ok(result)
}
pub(super) fn luns(xml: &str) -> Result<Vec<DiscoveredIscsiLun>, Error> {
    let doc = document(xml, "iscsi-target")?;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for row in rows(&doc, "LUN")? {
        let id = required(row, "SCSIid")?;
        if !crate::storage::storage_token(id) || !seen.insert(id.to_owned()) {
            return Err(invalid());
        }
        result.push(DiscoveredIscsiLun {
            scsi_id: id.into(),
            lun: number(row, "LUNid")?,
            size_bytes: number(row, "size")?,
            vendor: clean(value(row, "vendor")?.unwrap_or(""), 255),
            serial: clean(value(row, "serial")?.unwrap_or(""), 255),
        });
    }
    result.sort_by(|a, b| a.scsi_id.cmp(&b.scsi_id));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_projects_bounded_rows_and_ignores_secret_fields() {
        let e=exports("<nfs-exports><Export><Path>/images</Path><Accesslist>*</Accesslist><password>private</password></Export><Export><Path>iso</Path></Export></nfs-exports>").unwrap();
        assert_eq!(e[1].path, "/iso");
        assert!(!serde_json::to_string(&e).unwrap().contains("private"));
        let t=targets("<iscsi-target-iqns><TGT><IPAddress>[2001:db8::1]</IPAddress><Port>3261</Port><TargetIQN>iqn.2026-01.example:test</TargetIQN></TGT><TGT><TargetIQN>*</TargetIQN></TGT></iscsi-target-iqns>").unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].port, 3261);
        let l=luns("<iscsi-target><LUN><SCSIid>3600abc</SCSIid><size>4096</size><vendor>Example</vendor></LUN></iscsi-target>").unwrap();
        assert_eq!(l[0].lun, None);
        assert_eq!(l[0].size_bytes, Some(4096));
    }
    #[test]
    fn discovery_refuses_ambiguous_invalid_and_unbounded_payloads() {
        for xml in [
            "<!DOCTYPE a [<!ENTITY x 'value'>]><nfs-exports/>",
            "<nfs-exports><Export><Path>/one</Path><Path>/two</Path></Export></nfs-exports>",
            "<nfs-exports><Export><Path>../images</Path></Export></nfs-exports>",
            "<SRlist/>",
            "<nfs-exports><Export><Path>/one<!--break--><![CDATA[/two]]></Path></Export></nfs-exports>",
        ] {
            assert!(exports(xml).is_err());
        }
        let row = "<Export><Path>/images</Path></Export>";
        assert!(exports(&format!("<nfs-exports>{}</nfs-exports>", row.repeat(1001))).is_err());
        for port in ["0", "65536", "-1", "password"] {
            assert!(targets(&format!("<iscsi-target-iqns><TGT><IPAddress>192.0.2.2</IPAddress><Port>{port}</Port><TargetIQN>iqn.2026-01.example:test</TargetIQN></TGT></iscsi-target-iqns>")).is_err());
        }
        assert!(
            luns("<iscsi-target><LUN><SCSIid>id</SCSIid><size>-1</size></LUN></iscsi-target>")
                .is_err()
        );
        assert!(luns("<iscsi-target><LUN><SCSIid>id</SCSIid></LUN><LUN><SCSIid>id</SCSIid></LUN></iscsi-target>").is_err());
    }
}
