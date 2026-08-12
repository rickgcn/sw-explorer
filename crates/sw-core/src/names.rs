//! SGI product / image / subsystem naming.
//!
//! SGI names are dot-separated with a fixed arity:
//!
//! ```text
//! eoe             product       (one segment)
//! eoe.sw          image         (two segments)
//! eoe.sw.unix     subsystem     (three segments)
//! ```
//!
//! Because the arity is fixed, the qualified image name encoded by a
//! subsystem name is derivable from its first two segments; no
//! filesystem probing is needed. The exact *logical* image is not
//! derivable from a name — several logical images may share one
//! qualified name — and is identified by
//! [`crate::distribution::ImageKey`] instead.
//!
//! A qualified name is what the media call an object, not where the
//! object lives: the product segment of an image or subsystem name does
//! not necessarily name the product whose logical tree contains the
//! object (a product's IDB may reference a *foreign* subsystem).
//! Logical hierarchy identity lives in the keys of
//! [`crate::distribution`].

use crate::error::{Error, Result};

/// Validates one dotted-name segment.
///
/// Every segment of a product, image or subsystem name goes through this
/// validator: segments must be non-empty and free of `/` and NUL, so a
/// name can never acquire host path semantics (an image name becomes a
/// file name inside the distribution directory).
fn validate_segment(segment: &str) -> Result<()> {
    if segment.is_empty() || segment.contains(['/', '\0']) {
        return Err(Error::UnsafePath {
            path: segment.to_string(),
        });
    }
    Ok(())
}

/// A product name, e.g. `eoe`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProductName(String);

impl ProductName {
    /// Creates a product name from a single segment.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsafePath`] if the name is empty or contains
    /// `.`, `/` or NUL.
    pub fn new(name: &str) -> Result<Self> {
        validate_segment(name)?;
        if name.contains('.') {
            return Err(Error::UnsafePath {
                path: name.to_string(),
            });
        }
        Ok(ProductName(name.to_string()))
    }

    /// The name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProductName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An image name, e.g. `eoe.sw`.
///
/// This is a *qualified media name*: what the media call the image, and
/// also the file name of the image archive inside the distribution
/// directory. It is not a logical identity — several logical images of
/// one distribution may carry the same name, and the product segment
/// may name a different product than the one whose tree contains the
/// image. Logical identity is [`crate::distribution::ImageKey`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageName {
    product: ProductName,
    image: String,
}

impl ImageName {
    /// Parses a dotted image name.
    ///
    /// The name must be exactly `product.image`: two valid segments.
    /// This arity is fixed by the SGI format.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsafePath`] if the name does not have exactly
    /// two valid segments.
    pub fn parse(name: &str) -> Result<Self> {
        let invalid = || Error::UnsafePath {
            path: name.to_string(),
        };
        let mut segments = name.split('.');
        match (segments.next(), segments.next(), segments.next()) {
            (Some(product), Some(image), None) => {
                ImageName::from_parts(ProductName::new(product)?, image.to_string())
            }
            _ => Err(invalid()),
        }
    }

    /// Creates an image name from its parts.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsafePath`] if the image segment is empty or
    /// contains `.`, `/` or NUL.
    pub fn from_parts(product: ProductName, image: String) -> Result<Self> {
        validate_segment(&image)?;
        if image.contains('.') {
            return Err(Error::UnsafePath { path: image });
        }
        Ok(ImageName { product, image })
    }

    /// The product segment of the qualified name, e.g. `eoe` in
    /// `eoe.sw`.
    ///
    /// This is only what the media call the image's product; it does
    /// *not* guarantee the containing product of the logical image in a
    /// distribution: an IDB record of product `alpha` may name the
    /// foreign subsystem `beta.sw.unix`, growing a synthetic image
    /// `beta.sw` inside `alpha`'s tree whose product segment is `beta`.
    pub fn product(&self) -> &ProductName {
        &self.product
    }

    /// The image segment, e.g. `sw`.
    pub fn image(&self) -> &str {
        &self.image
    }

    /// The archive file name within the distribution directory, e.g.
    /// `eoe.sw`.
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.product, self.image)
    }
}

impl std::fmt::Display for ImageName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.product, self.image)
    }
}

/// A subsystem name, e.g. `eoe.sw.unix`.
///
/// Subsystems are the smallest installable unit of an SGI product.
///
/// This is a *qualified media name*: what the media call the subsystem.
/// It is not a logical identity — several logical subsystems of one
/// distribution may carry the same name, and the product segment may
/// name a different product than the one whose tree contains the
/// subsystem. Logical identity is
/// [`crate::distribution::SubsystemKey`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubsystemName {
    product: ProductName,
    image: String,
    subsystem: String,
}

impl SubsystemName {
    /// Parses a dotted subsystem name.
    ///
    /// The name must be exactly `product.image.subsystem`: three
    /// non-empty segments. This arity is fixed by the SGI format and was
    /// confirmed against all real IRIX 4.0.5 – 6.5 media.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsafePath`] if the name does not have exactly
    /// three valid segments.
    pub fn parse(name: &str) -> Result<Self> {
        let invalid = || Error::UnsafePath {
            path: name.to_string(),
        };
        let segments: Vec<&str> = name.split('.').collect();
        if segments.len() != 3 {
            return Err(invalid());
        }
        for segment in &segments {
            validate_segment(segment)?;
            if segment
                .chars()
                .any(|c| c.is_whitespace() || "()/=".contains(c))
            {
                return Err(invalid());
            }
        }
        Ok(SubsystemName {
            product: ProductName::new(segments[0])?,
            image: segments[1].to_string(),
            subsystem: segments[2].to_string(),
        })
    }

    /// The product segment of the qualified name, e.g. `eoe` in
    /// `eoe.sw.unix`.
    ///
    /// This is only what the media call the subsystem's product; it
    /// does *not* guarantee the containing product of the logical
    /// subsystem in a distribution (see [`ImageName::product`]).
    pub fn product(&self) -> &ProductName {
        &self.product
    }

    /// The image segment, e.g. `sw`.
    pub fn image(&self) -> &str {
        &self.image
    }

    /// The subsystem segment, e.g. `unix`.
    pub fn subsystem(&self) -> &str {
        &self.subsystem
    }

    /// The image this subsystem belongs to.
    pub fn image_name(&self) -> ImageName {
        // Both segments were validated by `parse`.
        ImageName {
            product: self.product.clone(),
            image: self.image.clone(),
        }
    }
}

impl std::fmt::Display for SubsystemName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.product, self.image, self.subsystem)
    }
}

/// Matches `text` against a shell-style wildcard pattern supporting `*`
/// (any run of characters) and `?` (a single character).
///
/// Used for subsystem rule patterns (`eoe*.sw*`) and path queries.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subsystem_names() {
        let name = SubsystemName::parse("eoe.sw.unix").unwrap();
        assert_eq!(name.product().as_str(), "eoe");
        assert_eq!(name.image(), "sw");
        assert_eq!(name.subsystem(), "unix");
        assert_eq!(name.image_name().file_name(), "eoe.sw");
        assert_eq!(name.to_string(), "eoe.sw.unix");
    }

    #[test]
    fn parses_image_names() {
        let name = ImageName::parse("eoe.sw").unwrap();
        assert_eq!(name.product().as_str(), "eoe");
        assert_eq!(name.image(), "sw");
        assert_eq!(name.file_name(), "eoe.sw");
        assert_eq!(name.to_string(), "eoe.sw");

        // Exactly two valid segments, no more, no fewer.
        assert!(ImageName::parse("eoe").is_err());
        assert!(ImageName::parse("eoe.sw.unix").is_err());
        assert!(ImageName::parse("eoe..sw").is_err());
        assert!(ImageName::parse("eoe.").is_err());
        assert!(ImageName::parse(".sw").is_err());
        assert!(ImageName::parse("eoe.s/w").is_err());
        assert!(ImageName::parse("eoe.s\0w").is_err());
    }

    #[test]
    fn rejects_malformed_names() {
        assert!(SubsystemName::parse("eoe").is_err());
        assert!(SubsystemName::parse("eoe.sw").is_err());
        assert!(SubsystemName::parse("eoe.sw.").is_err());
        assert!(SubsystemName::parse("eoe..unix").is_err());
        assert!(SubsystemName::parse("eoe.sw.foo.bar").is_err());
        assert!(SubsystemName::parse("sum(5488)").is_err());
        assert!(ProductName::new("eoe.sw").is_err());
    }

    #[test]
    fn rejects_host_path_semantics_in_segments() {
        assert!(ProductName::new("a/b").is_err());
        assert!(ProductName::new("a\0b").is_err());
        assert!(SubsystemName::parse("eoe.s/w.unix").is_err());
        assert!(SubsystemName::parse("eoe.sw.un\0ix").is_err());
        let product = ProductName::new("eoe").unwrap();
        assert!(ImageName::from_parts(product.clone(), "s/w".to_string()).is_err());
        assert!(ImageName::from_parts(product.clone(), "s.w".to_string()).is_err());
        assert!(ImageName::from_parts(product.clone(), String::new()).is_err());
        assert!(ImageName::from_parts(product, "sw".to_string()).is_ok());
    }

    #[test]
    fn wildcard_matching() {
        assert!(wildcard_match("eoe*.sw*", "eoe1.sw"));
        assert!(wildcard_match("*ge7.bin", "usr/sbin/ge7.bin"));
        assert!(wildcard_match("eoe.sw.uni?", "eoe.sw.unix"));
        assert!(!wildcard_match("eoe*.man*", "eoe1.sw"));
        assert!(wildcard_match("*", "anything"));
    }
}
