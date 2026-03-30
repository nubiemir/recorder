use libstrophe::Stanza;

pub fn find_all(element: Option<&Stanza>, selector: &str) -> Vec<Stanza> {
    let mut result = vec![];
    let selector: Vec<&str> = selector.split(">").collect();
    if let Some(ele) = element {
        for c in ele.children() {
            let ele = c.get_child_by_path(&selector);
            match ele {
                Some(stan) => {
                    result.push(stan.clone());
                }
                None => {
                    continue;
                }
            }
        }
    }
    result
}

pub fn find_first(element: Option<&Stanza>, selector: &str) -> Option<Stanza> {
    let selector: Vec<&str> = selector.split(">").collect();

    if let Some(ele) = element {
        for c in ele.children() {
            if let Some(stan) = c.get_child_by_path(&selector) {
                return Some(stan.clone());
            }
        }
        None
    } else {
        None
    }
}

pub fn exists(element: Option<&Stanza>, selector: &str) -> bool {
    find_all(element, selector).len() != 0
}
