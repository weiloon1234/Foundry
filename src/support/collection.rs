use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::iter::Sum;
use std::ops::Index;

use serde::{Deserialize, Serialize};

/// A lightweight wrapper around `Vec<T>` with ergonomic query and transform methods.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection<T> {
    items: Vec<T>,
}

// ---------------------------------------------------------------------------
// Constructors & conversions
// ---------------------------------------------------------------------------

impl<T> Collection<T> {
    /// Create an empty collection.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Create a collection from an existing `Vec<T>`.
    pub fn from_vec(items: Vec<T>) -> Self {
        Self { items }
    }

    /// Consume the collection and return the inner `Vec<T>`.
    pub fn into_vec(self) -> Vec<T> {
        self.items
    }

    /// Return a reference to the inner slice.
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    /// Return the number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Return `true` if the collection is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Return an iterator over references to the items.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    /// Clone the items into a new `Vec<T>`.
    pub fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.items.clone()
    }
}

// ---------------------------------------------------------------------------
// Trait implementations — interoperability
// ---------------------------------------------------------------------------

impl<T> From<Vec<T>> for Collection<T> {
    fn from(items: Vec<T>) -> Self {
        Self { items }
    }
}

impl<T> From<Collection<T>> for Vec<T> {
    fn from(value: Collection<T>) -> Self {
        value.items
    }
}

impl<T> FromIterator<T> for Collection<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            items: iter.into_iter().collect(),
        }
    }
}

impl<T> IntoIterator for Collection<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<T> AsRef<[T]> for Collection<T> {
    fn as_ref(&self) -> &[T] {
        &self.items
    }
}

impl<T> Index<usize> for Collection<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.items[index]
    }
}

impl<T> Default for Collection<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Access
// ---------------------------------------------------------------------------

impl<T> Collection<T> {
    /// Return a reference to the first item, or `None` if empty.
    pub fn first(&self) -> Option<&T> {
        self.items.first()
    }

    /// Return a reference to the last item, or `None` if empty.
    pub fn last(&self) -> Option<&T> {
        self.items.last()
    }

    /// Return a reference to the item at the given index, or `None` if out of bounds.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index)
    }
}

// ---------------------------------------------------------------------------
// Transform (all return new Collection)
// ---------------------------------------------------------------------------

impl<T> Collection<T> {
    /// Transform each element by reference, returning a new collection.
    pub fn map<U>(self, f: impl Fn(&T) -> U) -> Collection<U> {
        Collection::from_vec(self.items.iter().map(f).collect())
    }

    /// Transform each element by consuming it, returning a new collection.
    pub fn map_into<U>(self, f: impl Fn(T) -> U) -> Collection<U> {
        Collection::from_vec(self.items.into_iter().map(f).collect())
    }

    /// Keep only elements matching the predicate.
    pub fn filter(self, f: impl Fn(&T) -> bool) -> Collection<T> {
        Collection::from_vec(self.items.into_iter().filter(f).collect())
    }

    /// Remove elements matching the predicate.
    pub fn reject(self, f: impl Fn(&T) -> bool) -> Collection<T> {
        Collection::from_vec(self.items.into_iter().filter(|item| !f(item)).collect())
    }

    /// Map each element to a `Vec<U>`, then flatten into a single collection.
    pub fn flat_map<U>(self, f: impl Fn(T) -> Vec<U>) -> Collection<U> {
        Collection::from_vec(self.items.into_iter().flat_map(f).collect())
    }
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

impl<T> Collection<T> {
    /// Find the first element matching the predicate, returning a reference.
    pub fn find(&self, f: impl Fn(&T) -> bool) -> Option<&T> {
        self.items.iter().find(|item| f(item))
    }

    /// Find the first element matching the predicate, consuming the collection.
    pub fn first_where(self, f: impl Fn(&T) -> bool) -> Option<T> {
        self.items.into_iter().find(f)
    }

    /// Return `true` if any element matches the predicate.
    pub fn any(&self, f: impl Fn(&T) -> bool) -> bool {
        self.items.iter().any(f)
    }

    /// Return `true` if all elements match the predicate.
    pub fn all(&self, f: impl Fn(&T) -> bool) -> bool {
        self.items.iter().all(f)
    }

    /// Count how many elements match the predicate.
    pub fn count_where(&self, f: impl Fn(&T) -> bool) -> usize {
        self.items.iter().filter(|item| f(item)).count()
    }
}

// ---------------------------------------------------------------------------
// High-value backend helpers
// ---------------------------------------------------------------------------

impl<T> Collection<T> {
    /// Extract a derived value from each element.
    pub fn pluck<U>(self, f: impl Fn(&T) -> U) -> Collection<U> {
        Collection::from_vec(self.items.iter().map(f).collect())
    }

    /// Index all items by a key derived from each element.
    pub fn key_by<K: Eq + Hash>(self, f: impl Fn(&T) -> K) -> HashMap<K, T> {
        self.items
            .into_iter()
            .map(|item| {
                let key = f(&item);
                (key, item)
            })
            .collect()
    }

    /// Group items into `Collection<T>` buckets keyed by a derived value.
    pub fn group_by<K: Eq + Hash>(self, f: impl Fn(&T) -> K) -> HashMap<K, Collection<T>> {
        let mut map: HashMap<K, Vec<T>> = HashMap::new();
        for item in self.items {
            let key = f(&item);
            map.entry(key).or_default().push(item);
        }
        map.into_iter()
            .map(|(k, v)| (k, Collection::from_vec(v)))
            .collect()
    }

    /// Deduplicate by a key, preserving the first occurrence order.
    pub fn unique_by<K: Eq + Hash>(self, f: impl Fn(&T) -> K) -> Collection<T> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for item in self.items {
            let key = f(&item);
            if seen.insert(key) {
                result.push(item);
            }
        }
        Collection::from_vec(result)
    }

    /// Split into two collections: those matching and those not matching the predicate.
    pub fn partition_by(self, f: impl Fn(&T) -> bool) -> (Collection<T>, Collection<T>) {
        let (matching, rest): (Vec<T>, Vec<T>) = self.items.into_iter().partition(|item| f(item));
        (Collection::from_vec(matching), Collection::from_vec(rest))
    }

    /// Split into chunks of the given size. Returns an empty collection if size is 0
    /// or the collection is empty.
    pub fn chunk(self, size: usize) -> Collection<Collection<T>>
    where
        T: Clone,
    {
        if size == 0 || self.items.is_empty() {
            return Collection::new();
        }
        Collection::from_vec(
            self.items
                .chunks(size)
                .map(|chunk| Collection::from_vec(chunk.to_vec()))
                .collect(),
        )
    }
}

// ---------------------------------------------------------------------------
// Ordering (in-place)
// ---------------------------------------------------------------------------

impl<T> Collection<T> {
    /// Sort in-place using a comparator function.
    pub fn sort_by(&mut self, f: impl Fn(&T, &T) -> std::cmp::Ordering) {
        self.items.sort_by(f);
    }

    /// Sort in-place by a derived key.
    pub fn sort_by_key<K: Ord>(&mut self, f: impl Fn(&T) -> K) {
        self.items.sort_by_key(f);
    }

    /// Reverse the order of elements in-place.
    pub fn reverse(&mut self) {
        self.items.reverse();
    }
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

impl<T> Collection<T> {
    /// Sum a derived value across all elements.
    pub fn sum_by<U: Sum>(self, f: impl Fn(&T) -> U) -> U {
        self.items.iter().map(f).sum()
    }

    /// Find the minimum derived value, or `None` if empty.
    pub fn min_by<U: Ord>(self, f: impl Fn(&T) -> U) -> Option<U> {
        self.items.iter().map(f).min()
    }

    /// Find the maximum derived value, or `None` if empty.
    pub fn max_by<U: Ord>(self, f: impl Fn(&T) -> U) -> Option<U> {
        self.items.iter().map(f).max()
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

impl<T> Collection<T> {
    /// Take the first `n` elements.
    pub fn take(self, n: usize) -> Collection<T> {
        Collection::from_vec(self.items.into_iter().take(n).collect())
    }

    /// Skip the first `n` elements.
    pub fn skip(self, n: usize) -> Collection<T> {
        Collection::from_vec(self.items.into_iter().skip(n).collect())
    }

    /// Consume each element with a side-effect function.
    pub fn for_each(self, f: impl FnMut(T)) {
        self.items.into_iter().for_each(f);
    }

    /// Inspect the collection without consuming it, then return it for further chaining.
    pub fn tap(self, mut f: impl FnMut(&Collection<T>)) -> Collection<T> {
        f(&self);
        self
    }

    /// Pass the collection through a transform function and return the result.
    pub fn pipe(self, f: impl Fn(Collection<T>) -> Collection<T>) -> Collection<T> {
        f(self)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::Collection;

    // -- Constructors & conversions --

    #[test]
    fn new_creates_empty_collection() {
        let c: Collection<i32> = Collection::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn from_vec_and_into_vec_roundtrip() {
        let original = vec![1, 2, 3];
        let c = Collection::from_vec(original.clone());
        assert_eq!(c.into_vec(), original);
    }

    #[test]
    fn as_slice() {
        let c = Collection::from_vec(vec![10, 20]);
        assert_eq!(c.as_slice(), &[10, 20]);
    }

    #[test]
    fn to_vec_clones() {
        let c = Collection::from_vec(vec![1, 2]);
        let v = c.to_vec();
        assert_eq!(v, vec![1, 2]);
        // original still usable
        assert_eq!(c.len(), 2);
    }

    // -- Trait implementations --

    #[test]
    fn from_vec_trait() {
        let c: Collection<i32> = Collection::from(vec![1, 2]);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn into_vec_trait() {
        let c = Collection::from_vec(vec![5, 6]);
        let v: Vec<i32> = c.into();
        assert_eq!(v, vec![5, 6]);
    }

    #[test]
    fn from_iterator() {
        let c: Collection<i32> = vec![1, 2, 3].into_iter().filter(|&x| x > 1).collect();
        assert_eq!(c.into_vec(), vec![2, 3]);
    }

    #[test]
    fn into_iterator() {
        let c = Collection::from_vec(vec![1, 2, 3]);
        let sum: i32 = c.into_iter().sum();
        assert_eq!(sum, 6);
    }

    #[test]
    fn as_ref() {
        let c = Collection::from_vec(vec![7, 8]);
        let slice: &[i32] = c.as_ref();
        assert_eq!(slice, &[7, 8]);
    }

    #[test]
    fn default_is_empty() {
        let c: Collection<String> = Collection::default();
        assert!(c.is_empty());
    }

    // -- Access --

    #[test]
    fn first_last_get_on_non_empty() {
        let c = Collection::from_vec(vec![10, 20, 30]);
        assert_eq!(c.first(), Some(&10));
        assert_eq!(c.last(), Some(&30));
        assert_eq!(c.get(1), Some(&20));
    }

    #[test]
    fn first_last_get_on_empty() {
        let c: Collection<i32> = Collection::new();
        assert_eq!(c.first(), None);
        assert_eq!(c.last(), None);
        assert_eq!(c.get(0), None);
    }

    // -- Transform --

    #[test]
    fn map_by_reference() {
        let c = Collection::from_vec(vec![1, 2, 3]);
        let doubled = c.map(|x| x * 2);
        assert_eq!(doubled.into_vec(), vec![2, 4, 6]);
    }

    #[test]
    fn map_into_consuming() {
        let c = Collection::from_vec(vec![1, 2, 3]);
        let as_strings = c.map_into(|x| x.to_string());
        assert_eq!(
            as_strings.into_vec(),
            vec!["1".to_string(), "2".to_string(), "3".to_string()]
        );
    }

    #[test]
    fn filter_keeps_matches() {
        let c = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let evens = c.filter(|x| *x % 2 == 0);
        assert_eq!(evens.into_vec(), vec![2, 4]);
    }

    #[test]
    fn reject_removes_matches() {
        let c = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let odds = c.reject(|x| *x % 2 == 0);
        assert_eq!(odds.into_vec(), vec![1, 3, 5]);
    }

    #[test]
    fn flat_map_flattens() {
        let c = Collection::from_vec(vec![1, 2, 3]);
        let expanded = c.flat_map(|x| vec![x, x * 10]);
        assert_eq!(expanded.into_vec(), vec![1, 10, 2, 20, 3, 30]);
    }

    // -- Query --

    #[test]
    fn find_returns_reference() {
        let c = Collection::from_vec(vec![10, 20, 30]);
        assert_eq!(c.find(|&x| x == 20), Some(&20));
    }

    #[test]
    fn find_returns_none_when_no_match() {
        let c = Collection::from_vec(vec![1, 2, 3]);
        assert_eq!(c.find(|&x| x == 99), None);
    }

    #[test]
    fn first_where_returns_owned() {
        let c = Collection::from_vec(vec![10, 20, 30]);
        assert_eq!(c.first_where(|x| *x > 15), Some(20));
    }

    #[test]
    fn first_where_returns_none() {
        let c = Collection::from_vec(vec![1, 2]);
        assert_eq!(c.first_where(|x| *x > 100), None);
    }

    #[test]
    fn any_and_all() {
        let c = Collection::from_vec(vec![2, 4, 6]);
        assert!(c.any(|x| *x == 4));
        assert!(!c.any(|x| *x == 5));
        assert!(c.all(|x| *x % 2 == 0));
        assert!(!c.all(|x| *x > 5));
    }

    #[test]
    fn count_where() {
        let c = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        assert_eq!(c.count_where(|x| *x % 2 != 0), 3);
    }

    // -- High-value backend helpers --

    #[test]
    fn pluck_extracts_field() {
        #[derive(Clone)]
        struct User {
            name: String,
            age: u32,
        }
        let users = Collection::from_vec(vec![
            User {
                name: "Alice".into(),
                age: 30,
            },
            User {
                name: "Bob".into(),
                age: 25,
            },
        ]);
        let names = users.pluck(|u| u.name.clone());
        assert_eq!(
            names.into_vec(),
            vec!["Alice".to_string(), "Bob".to_string()]
        );
        let ages = Collection::from_vec(vec![
            User {
                name: "Alice".into(),
                age: 30,
            },
            User {
                name: "Bob".into(),
                age: 25,
            },
        ])
        .pluck(|u| u.age);
        assert_eq!(ages.into_vec(), vec![30, 25]);
    }

    #[test]
    fn key_by_indexes_elements() {
        let c = Collection::from_vec(vec![(1, "a"), (2, "b"), (3, "c")]);
        let map = c.key_by(|(k, _)| *k);
        assert_eq!(map.get(&2).unwrap().1, "b");
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn group_by_buckets() {
        let c = Collection::from_vec(vec![1, 2, 3, 4, 5, 6]);
        let groups = c.group_by(|x| *x % 2);
        assert_eq!(groups.get(&0).unwrap().len(), 3); // 2, 4, 6
        assert_eq!(groups.get(&1).unwrap().len(), 3); // 1, 3, 5
    }

    #[test]
    fn unique_by_deduplicates_preserving_order() {
        let c = Collection::from_vec(vec![(1, "a"), (2, "b"), (1, "c"), (3, "d"), (2, "e")]);
        let unique = c.unique_by(|(k, _)| *k);
        assert_eq!(unique.into_vec(), vec![(1, "a"), (2, "b"), (3, "d")]);
    }

    #[test]
    fn partition_by_splits() {
        let c = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let (evens, odds) = c.partition_by(|x| *x % 2 == 0);
        assert_eq!(evens.into_vec(), vec![2, 4]);
        assert_eq!(odds.into_vec(), vec![1, 3, 5]);
    }

    #[test]
    fn chunk_splits_evenly() {
        let c = Collection::from_vec(vec![1, 2, 3, 4, 5, 6]);
        let chunks = c.chunk(2);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.get(0).unwrap().as_slice(), &[1, 2]);
        assert_eq!(chunks.get(2).unwrap().as_slice(), &[5, 6]);
    }

    #[test]
    fn chunk_with_remainder() {
        let c = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let chunks = c.chunk(2);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.get(2).unwrap().as_slice(), &[5]);
    }

    #[test]
    fn chunk_empty_returns_empty() {
        let c: Collection<i32> = Collection::new();
        let chunks = c.chunk(3);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_size_zero_returns_empty() {
        let c = Collection::from_vec(vec![1, 2, 3]);
        let chunks = c.chunk(0);
        assert!(chunks.is_empty());
    }

    // -- Ordering --

    #[test]
    fn sort_by_orders_in_place() {
        let mut c = Collection::from_vec(vec![3, 1, 2]);
        c.sort_by(|a, b| a.cmp(b));
        assert_eq!(c.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn sort_by_key_orders_in_place() {
        let mut c = Collection::from_vec(vec![(2, "b"), (1, "a"), (3, "c")]);
        c.sort_by_key(|(k, _)| *k);
        assert_eq!(c.get(0).unwrap().1, "a");
        assert_eq!(c.get(2).unwrap().1, "c");
    }

    #[test]
    fn reverse_in_place() {
        let mut c = Collection::from_vec(vec![1, 2, 3]);
        c.reverse();
        assert_eq!(c.as_slice(), &[3, 2, 1]);
    }

    // -- Aggregation --

    #[test]
    fn sum_by_adds_up() {
        let c = Collection::from_vec(vec![1, 2, 3, 4]);
        assert_eq!(c.sum_by(|x| *x), 10);
    }

    #[test]
    fn min_by_returns_minimum() {
        let c = Collection::from_vec(vec![5, 2, 8, 1, 9]);
        assert_eq!(c.min_by(|x| *x), Some(1));
    }

    #[test]
    fn min_by_empty_returns_none() {
        let c: Collection<i32> = Collection::new();
        assert_eq!(c.min_by(|x| *x), None);
    }

    #[test]
    fn max_by_returns_maximum() {
        let c = Collection::from_vec(vec![5, 2, 8, 1, 9]);
        assert_eq!(c.max_by(|x| *x), Some(9));
    }

    #[test]
    fn max_by_empty_returns_none() {
        let c: Collection<i32> = Collection::new();
        assert_eq!(c.max_by(|x| *x), None);
    }

    // -- Utilities --

    #[test]
    fn take_first_n() {
        let c = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let taken = c.take(3);
        assert_eq!(taken.into_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn take_more_than_len() {
        let c = Collection::from_vec(vec![1, 2]);
        let taken = c.take(10);
        assert_eq!(taken.into_vec(), vec![1, 2]);
    }

    #[test]
    fn skip_first_n() {
        let c = Collection::from_vec(vec![1, 2, 3, 4, 5]);
        let remaining = c.skip(2);
        assert_eq!(remaining.into_vec(), vec![3, 4, 5]);
    }

    #[test]
    fn skip_more_than_len() {
        let c = Collection::from_vec(vec![1, 2]);
        let remaining = c.skip(10);
        assert!(remaining.is_empty());
    }

    #[test]
    fn for_each_consumes() {
        let c = Collection::from_vec(vec![1, 2, 3]);
        let mut sum = 0;
        c.for_each(|x| sum += x);
        assert_eq!(sum, 6);
    }

    #[test]
    fn tap_inspects_without_consuming() {
        let c = Collection::from_vec(vec![1, 2, 3]);
        let mut seen_len = 0;
        let result = c.tap(|c| seen_len = c.len());
        assert_eq!(seen_len, 3);
        assert_eq!(result.into_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn pipe_transforms() {
        let c = Collection::from_vec(vec![1, 2, 3]);
        let result = c.pipe(|c| c.filter(|x| *x > 1));
        assert_eq!(result.into_vec(), vec![2, 3]);
    }
}
