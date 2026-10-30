//! Local search state; has no transport or resolver access.
use yami_render::native::SearchMatch;
#[derive(Default)]
pub struct Find {
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub current: usize,
}
impl Find {
    pub fn step(&mut self, backwards: bool) {
        let n = self.matches.len();
        self.current = if n == 0 {
            0
        } else if backwards {
            (self.current + n - 1) % n
        } else {
            (self.current + 1) % n
        };
    }
    pub fn selected(&self) -> Option<&SearchMatch> {
        self.matches.get(self.current)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_and_bidirectional_wrap() {
        let mut find = Find::default();
        find.step(true);
        assert!(find.selected().is_none());
        find.matches = vec![SearchMatch { rects: vec![] }; 3];
        find.step(true);
        assert_eq!(find.current, 2);
        find.step(false);
        assert_eq!(find.current, 0);
    }
}
