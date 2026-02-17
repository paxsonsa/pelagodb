use super::compiler::CompiledBlock;

/// Generate human-readable query plan output
pub fn explain(blocks: &[CompiledBlock]) -> String {
    let mut output = String::new();
    output.push_str("Query Plan:\n");
    output.push_str("===========================================\n\n");

    for (i, block) in blocks.iter().enumerate() {
        output.push_str(&format!("Block {} ", i + 1));
        match block {
            CompiledBlock::PointLookup {
                block_name,
                entity_type,
                node_id,
                fields,
            } => {
                output.push_str(&format!("\"{}\":\n", block_name));
                output.push_str("  Strategy: point_lookup\n");
                output.push_str(&format!("  Type: {}\n", entity_type));
                output.push_str(&format!("  ID: {}\n", node_id));
                output.push_str(&format!("  Fields: [{}]\n", fields.join(", ")));
                output.push_str("  Estimated cost: 1.0\n");
                output.push_str("  Estimated rows: 1\n");
            }
            CompiledBlock::FindNodes {
                block_name,
                entity_type,
                cel_expression,
                fields,
                limit,
                offset,
            } => {
                output.push_str(&format!("\"{}\":\n", block_name));
                let strategy = if cel_expression.is_some() {
                    "index_scan"
                } else {
                    "full_scan"
                };
                output.push_str(&format!("  Strategy: {}\n", strategy));
                output.push_str(&format!("  Type: {}\n", entity_type));
                if let Some(cel) = cel_expression {
                    output.push_str(&format!("  Filter: {}\n", cel));
                }
                output.push_str(&format!("  Fields: [{}]\n", fields.join(", ")));
                if let Some(lim) = limit {
                    output.push_str(&format!("  Limit: {}\n", lim));
                }
                if let Some(off) = offset {
                    output.push_str(&format!("  Offset: {}\n", off));
                }
            }
            CompiledBlock::Traverse {
                block_name,
                start_entity_type,
                steps,
                max_depth,
                max_results,
                cascade,
                ..
            } => {
                output.push_str(&format!("\"{}\":\n", block_name));
                output.push_str("  Strategy: traversal\n");
                output.push_str(&format!("  Start: {}\n", start_entity_type));
                output.push_str(&format!("  Max depth: {}\n", max_depth));
                output.push_str(&format!("  Max results: {}\n", max_results));
                if *cascade {
                    output.push_str("  Cascade: true\n");
                }
                for (j, step) in steps.iter().enumerate() {
                    output.push_str(&format!("  Step {}:\n", j + 1));
                    output.push_str(&format!(
                        "    Edge: {} {}\n",
                        step.edge_type, step.direction
                    ));
                    if let Some(ef) = &step.edge_filter {
                        output.push_str(&format!("    Edge filter: {}\n", ef));
                    }
                    if let Some(nf) = &step.node_filter {
                        output.push_str(&format!("    Node filter: {}\n", nf));
                    }
                    if !step.fields.is_empty() {
                        output.push_str(&format!("    Fields: [{}]\n", step.fields.join(", ")));
                    }
                    if !step.edge_fields.is_empty() {
                        output.push_str(&format!(
                            "    Edge fields: [{}]\n",
                            step.edge_fields.join(", ")
                        ));
                    }
                    if let Some(lim) = step.per_node_limit {
                        output.push_str(&format!("    Per-node limit: {}\n", lim));
                    }
                    if let Some(sort) = &step.sort {
                        let order = if sort.descending { "desc" } else { "asc" };
                        let target = if sort.on_edge { " (on edge)" } else { "" };
                        output.push_str(&format!("    Sort: {} {}{}\n", sort.field, order, target));
                    }
                }
            }
            CompiledBlock::VariableRef {
                block_name,
                variable,
                filter,
                fields,
            } => {
                output.push_str(&format!("\"{}\":\n", block_name));
                output.push_str("  Strategy: variable_ref\n");
                output.push_str(&format!("  Variable: ${}\n", variable));
                if let Some(f) = filter {
                    output.push_str(&format!("  Filter: {}\n", f));
                }
                if !fields.is_empty() {
                    output.push_str(&format!("  Fields: [{}]\n", fields.join(", ")));
                }
            }
        }
        output.push('\n');
    }

    output
}
