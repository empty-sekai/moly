//! 家具挂点配对律。
//!
//! 源路径：`FixtureView.AttachActionPoints` 遍历家具子树的全部
//! Transform，把 `loc_start` 节点和它的 `loc_end` 对节点配成一对，
//! 配不齐就抛异常——fail-closed，不静默丢挂点。
//!
//! 钉在源方法体上的两个形状：
//! - 配对键 = 节点名**小写化**后做 `loc_start → loc_end` 的全量替换；
//! - 查找是在**原名**上做精确等值——不是再小写一次。所以一个叫
//!   `Loc_End` 的节点配不上键 `loc_end`：源就是这么写的，只有
//!   原名本来就是小写的对节点才会命中。这里照抄，不「修」它。
//!
//! 小写化用 Unicode 默认折叠；源用的是当前区域的小写。本域的
//! 节点名是 ASCII，两种折叠在 ASCII 上逐字相同，不构成分叉。

/// 一个 `loc_start` 节点的配对键：小写名里的 `loc_start` 全量换成
/// `loc_end`。名字不含 `loc_start` 的节点返回 None（不参与配对）。
pub fn pair_key(node_name: &str) -> Option<String> {
    let lowered = node_name.to_lowercase();
    if lowered.contains("loc_start") {
        Some(lowered.replace("loc_start", "loc_end"))
    } else {
        None
    }
}

/// 一棵子树的挂点配对：每个 `loc_start` 名节点找出它的对节点。
///
/// 返回 `(start 名, end 名)` 的列表；任一 start 找不到对节点时
/// 整体报错（源在这里抛异常，报错文案带上缺对的名字）。
pub fn pair_attach_points<S: AsRef<str>>(
    names: &[S],
) -> Result<Vec<(String, String)>, String> {
    let mut pairs = Vec::new();
    for name in names {
        let name = name.as_ref();
        if let Some(key) = pair_key(name) {
            // 精确等值查找，对象是原名不是小写名——见模块注释。
            if names
                .iter()
                .any(|candidate| candidate.as_ref() == key)
            {
                pairs.push((name.to_string(), key));
            } else {
                return Err(format!(
                    "fixture attach point {name:?} has no matching end point {key:?}"
                ));
            }
        }
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_nodes_pair_with_lowercase_end_nodes() {
        let names = ["chair_loc_start", "chair_loc_end", "body"];
        let pairs = pair_attach_points(&names).unwrap();
        assert_eq!(
            pairs,
            vec![("chair_loc_start".to_string(), "chair_loc_end".to_string())]
        );
        // 不含 start 的节点不参与、也不报错。
        let pairs = pair_attach_points(&["chair_loc_end", "body"]).unwrap();
        assert!(pairs.is_empty());
    }

    #[test]
    fn mixed_case_start_still_pairs_with_lowercase_end() {
        // start 名的大写在键计算时被小写化：Loc_Start 的键是 loc_end。
        let names = ["Loc_Start", "loc_end"];
        let pairs = pair_attach_points(&names).unwrap();
        assert_eq!(
            pairs,
            vec![("Loc_Start".to_string(), "loc_end".to_string())]
        );
    }

    #[test]
    fn mixed_case_end_does_not_match() {
        // 对节点原名是 Loc_End（混合大小写）：与键 loc_end 不等值，
        // 配不上——源在原名上做精确等值，不是再小写一次。
        // 若实现把两边都小写再比，这一臂红。
        assert!(pair_attach_points(&["loc_start", "Loc_End"]).is_err());
    }

    #[test]
    fn missing_end_is_loud() {
        // 只有 start 没有 end：整体报错（fail-closed），不返回半张配对表。
        let err = pair_attach_points(&["chair_loc_start"]).unwrap_err();
        assert!(err.contains("chair_loc_start"));
        assert!(err.contains("chair_loc_end"));
    }

    #[test]
    fn multiple_start_occurrences_all_become_end() {
        // 一个名字里 loc_start 出现两次：替换是全量的，键里两个都是 end。
        assert_eq!(
            pair_key("loc_start_loc_start").unwrap(),
            "loc_end_loc_end"
        );
        let names = ["loc_start_loc_start", "loc_end_loc_end"];
        assert!(pair_attach_points(&names).is_ok());
    }

    #[test]
    fn unrelated_names_are_untouched() {
        // 没有 loc_start 字样的名字不进配对，也不被误报。
        assert_eq!(pair_key("locator"), None);
        assert_eq!(pair_key("loc_end_only"), None);
        assert_eq!(pair_key(""), None);
        assert!(pair_attach_points(&["armature", "mesh"]).is_ok());
    }
}
