use std::collections::BTreeMap;

use crate::foundation::{Error, Result};

use super::javascript::to_camel_case_identifier_with_context;

#[derive(Default)]
pub(crate) struct DottedIdTreeNode {
    pub(crate) value: Option<String>,
    pub(crate) children: BTreeMap<String, DottedIdTreeChild>,
}

pub(crate) struct DottedIdTreeChild {
    pub(crate) segment: String,
    pub(crate) node: DottedIdTreeNode,
}

fn first_dotted_id(node: &DottedIdTreeNode) -> Option<&str> {
    node.value.as_deref().or_else(|| {
        node.children
            .values()
            .find_map(|child| first_dotted_id(&child.node))
    })
}

fn insert_dotted_id_node(
    node: &mut DottedIdTreeNode,
    id: &str,
    segments: &[&str],
    context: &str,
    id_label: &str,
) -> Result<()> {
    let Some((segment, remaining)) = segments.split_first() else {
        if let Some(existing) = &node.value {
            return Err(Error::message(format!(
                "{context} contains duplicate {id_label} `{existing}`"
            )));
        }
        if let Some(existing) = first_dotted_id(node) {
            return Err(Error::message(format!(
                "{context} cannot group {id_label} `{id}` because it is also a prefix of `{existing}`"
            )));
        }

        node.value = Some(id.to_string());
        return Ok(());
    };

    if let Some(existing) = &node.value {
        return Err(Error::message(format!(
            "{context} cannot group {id_label} `{id}` because `{existing}` is already a {id_label}"
        )));
    }
    if segment.is_empty() {
        return Err(Error::message(format!(
            "{context} requires non-empty {id_label} segments; got `{id}`"
        )));
    }

    let property = to_camel_case_identifier_with_context(segment, context)?;
    if let Some(child) = node.children.get_mut(&property) {
        if child.segment != *segment {
            return Err(Error::message(format!(
                "{context} has {id_label} segments `{}` and `{segment}` that both normalize to `{property}`",
                child.segment
            )));
        }

        return insert_dotted_id_node(&mut child.node, id, remaining, context, id_label);
    }

    node.children.insert(
        property.clone(),
        DottedIdTreeChild {
            segment: (*segment).to_string(),
            node: DottedIdTreeNode::default(),
        },
    );
    let child = node
        .children
        .get_mut(&property)
        .expect("just inserted dotted id segment");
    insert_dotted_id_node(&mut child.node, id, remaining, context, id_label)
}

pub(crate) fn insert_dotted_id(
    node: &mut DottedIdTreeNode,
    id: &str,
    context: &str,
    id_label: &str,
) -> Result<()> {
    let segments = id.split('.').collect::<Vec<_>>();
    insert_dotted_id_node(node, id, &segments, context, id_label)
}

pub(crate) fn dotted_id_tree<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    context: &str,
    id_label: &str,
) -> Result<DottedIdTreeNode> {
    let mut root = DottedIdTreeNode::default();

    for id in ids {
        insert_dotted_id(&mut root, id, context, id_label)?;
    }

    Ok(root)
}
