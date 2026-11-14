//! Splits the HT-Demucs graph so most of it can run on the GPU.
//!
//! DirectML runs out of video memory on the model's final inverse-STFT step
//! (two very large ConvTranspose ops), even on an 8 GB card. That step is cheap on
//! the CPU, so the graph is cut at the inverse-STFT input:
//!   part A = everything else            -> GPU (DirectML)
//!   part B = inverse STFT and what follows (adding the time branch, reshaping)
//!            -> CPU
//! Tensors that flow from A into B become A's outputs and B's inputs.
//! The split is done once and the two parts are cached next to the model.

use onnx_protobuf::{type_proto, GraphProto, Message, ModelProto, TypeProto, ValueInfoProto};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The tensor where the inverse STFT starts.
const CUT: &str = "/Pad_2_output_0";

/// Splits `model` into (A, B, boundary tensor names). B's inputs get the element
/// types in `types` (filled in after A has run once); untyped names are left out.
pub fn split(model: &ModelProto) -> Result<(ModelProto, ModelProto, Vec<String>), String> {
    let g = model.graph.as_ref().ok_or("model has no graph")?;
    let producer: HashMap<&str, usize> = g.node.iter().enumerate().flat_map(|(i, n)| n.output.iter().map(move |o| (o.as_str(), i))).collect();
    if !producer.contains_key(CUT) {
        return Err(format!("cut tensor {} not found", CUT));
    }
    // Consumers of each tensor.
    let mut consumers: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, n) in g.node.iter().enumerate() {
        for inp in &n.input {
            if !inp.is_empty() {
                consumers.entry(inp.as_str()).or_default().push(i);
            }
        }
    }
    // B = everything downstream of the cut.
    let mut in_b: HashSet<usize> = HashSet::new();
    let mut stack: Vec<&str> = vec![CUT];
    while let Some(t) = stack.pop() {
        for &c in consumers.get(t).map(|v| v.as_slice()).unwrap_or(&[]) {
            if in_b.insert(c) {
                for o in &g.node[c].output {
                    stack.push(o);
                }
            }
        }
    }
    // Pull small constant-only producers into B too, so constants are not boundary tensors.
    loop {
        let mut added = false;
        let needed: Vec<String> = in_b.iter().flat_map(|&i| g.node[i].input.clone()).collect();
        for t in needed {
            if let Some(&p) = producer.get(t.as_str()) {
                if !in_b.contains(&p) && (g.node[p].op_type == "Constant" || (g.node[p].op_type == "Cast" && g.node[p].input.iter().all(|x| producer.get(x.as_str()).map(|&q| g.node[q].op_type == "Constant").unwrap_or(false)))) {
                    in_b.insert(p);
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }
    let inits: HashSet<&str> = g.initializer.iter().map(|t| t.name.as_str()).collect();
    let graph_inputs: HashSet<&str> = g.input.iter().map(|v| v.name.as_str()).collect();
    // Boundary: tensors B reads that are made outside B (by A or the graph input).
    let mut boundary: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let mut b_idx: Vec<usize> = in_b.iter().copied().collect();
    b_idx.sort();
    for &i in &b_idx {
        for t in &g.node[i].input {
            if t.is_empty() || inits.contains(t.as_str()) {
                continue;
            }
            let outside = match producer.get(t.as_str()) {
                Some(p) => !in_b.contains(p),
                None => graph_inputs.contains(t.as_str()),
            };
            if outside && seen.insert(t.clone()) {
                boundary.push(t.clone());
            }
        }
    }
    // A = nodes needed (backwards) for the boundary tensors.
    let mut in_a: HashSet<usize> = HashSet::new();
    let mut stack: Vec<String> = boundary.clone();
    while let Some(t) = stack.pop() {
        if let Some(&p) = producer.get(t.as_str()) {
            if !in_b.contains(&p) && in_a.insert(p) {
                for i in &g.node[p].input {
                    if !i.is_empty() {
                        stack.push(i.clone());
                    }
                }
            }
        }
    }
    let make = |nodes: &HashSet<usize>, inputs: Vec<ValueInfoProto>, outputs: Vec<ValueInfoProto>| -> ModelProto {
        let mut idx: Vec<usize> = nodes.iter().copied().collect();
        idx.sort();
        let used: HashSet<&str> = idx.iter().flat_map(|&i| g.node[i].input.iter().map(|s| s.as_str())).collect();
        let mut ng = GraphProto::new();
        ng.name = g.name.clone();
        ng.node = idx.iter().map(|&i| g.node[i].clone()).collect();
        ng.initializer = g.initializer.iter().filter(|t| used.contains(t.name.as_str())).cloned().collect();
        ng.input = inputs;
        ng.output = outputs;
        let mut m = model.clone();
        m.graph = protobuf::MessageField::some(ng);
        m
    };
    let named = |n: &str| {
        let mut v = ValueInfoProto::new();
        v.name = n.to_string();
        v
    };
    let a = make(&in_a, g.input.clone(), boundary.iter().filter(|b| !graph_inputs.contains(b.as_str())).map(|b| named(b)).collect());
    let b = make(&in_b, Vec::new(), g.output.clone());
    Ok((a, b, boundary))
}

/// Gives B its inputs with element types (ONNX TensorProto codes: 1 f32, 7 i64, 10 f16 ...).
pub fn type_inputs(b: &mut ModelProto, inputs: &[(String, i32)]) {
    let g = b.graph.as_mut().unwrap();
    g.input = inputs
        .iter()
        .map(|(n, ty)| {
            let mut t = type_proto::Tensor::new();
            t.elem_type = *ty;
            let mut tp = TypeProto::new();
            tp.value = Some(type_proto::Value::TensorType(t));
            let mut v = ValueInfoProto::new();
            v.name = n.clone();
            v.type_ = protobuf::MessageField::some(tp);
            v
        })
        .collect();
}

pub fn load(path: &Path) -> Result<ModelProto, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    ModelProto::parse_from_bytes(&bytes).map_err(|e| e.to_string())
}

pub fn save(m: &ModelProto, path: &Path) -> Result<(), String> {
    let bytes = m.write_to_bytes().map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}
