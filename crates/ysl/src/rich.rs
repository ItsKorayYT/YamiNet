use crate::Color;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Typography {
    pub size: u16,
    pub weight: u16,
    pub align: Align,
    pub line_height: u16,
    pub gap: u16,
}
impl Typography {
    pub fn valid(self) -> bool {
        (12..=72).contains(&self.size)
            && matches!(self.weight, 400 | 600)
            && (100..=220).contains(&self.line_height)
            && self.gap <= 64
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardStyle {
    pub paper: Color,
    pub ink: Color,
    pub border: Color,
    pub border_width: u16,
    pub radius: u16,
    pub padding: u16,
    pub gap: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageStyle {
    pub width: u16,
    pub height: u16,
    pub align: Align,
    pub gap: u16,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichStyle {
    pub width: u16,
    pub padding: u16,
    pub text: Typography,
    pub heading: Typography,
    pub section: Typography,
    pub link: Typography,
    pub card: CardStyle,
    pub image: ImageStyle,
    pub row_gap: u16,
    pub row_min_width: u16,
    pub hover: Color,
    pub active: Color,
}
impl Default for RichStyle {
    fn default() -> Self {
        let text = Typography {
            size: 18,
            weight: 400,
            align: Align::Left,
            line_height: 150,
            gap: 12,
        };
        Self {
            width: 960,
            padding: 24,
            text,
            heading: Typography {
                size: 36,
                weight: 600,
                gap: 16,
                ..text
            },
            section: Typography {
                size: 23,
                weight: 600,
                gap: 12,
                ..text
            },
            link: Typography {
                weight: 600,
                gap: 10,
                ..text
            },
            card: CardStyle {
                paper: Color(25, 37, 49),
                ink: Color(216, 226, 236),
                border: Color(64, 89, 112),
                border_width: 1,
                radius: 14,
                padding: 20,
                gap: 20,
            },
            image: ImageStyle {
                width: 800,
                height: 1024,
                align: Align::Center,
                gap: 16,
            },
            row_gap: 20,
            row_min_width: 240,
            hover: Color(255, 213, 160),
            active: Color(97, 218, 251),
        }
    }
}
impl RichStyle {
    pub fn valid(&self) -> bool {
        (240..=1440).contains(&self.width)
            && self.padding <= 64
            && [self.text, self.heading, self.section, self.link]
                .into_iter()
                .all(Typography::valid)
            && self.card.border_width <= 4
            && self.card.radius <= 32
            && self.card.padding <= 48
            && self.card.gap <= 64
            && (16..=1440).contains(&self.image.width)
            && (16..=1536).contains(&self.image.height)
            && self.image.gap <= 64
            && self.row_gap <= 64
            && (160..=640).contains(&self.row_min_width)
    }
    pub(crate) fn set(&mut self, group: &str, name: &str, value: &str) -> Result<bool, String> {
        let number = |min: u16, max: u16| -> Result<u16, String> {
            let n = value
                .parse::<u16>()
                .map_err(|_| "expected bounded unsigned integer")?;
            if !(min..=max).contains(&n) {
                return Err(format!("number must be {min}..{max}"));
            }
            Ok(n)
        };
        let align = || -> Result<Align, String> {
            match value {
                "left" => Ok(Align::Left),
                "center" => Ok(Align::Center),
                "right" => Ok(Align::Right),
                _ => Err("alignment must be left, center or right".into()),
            }
        };
        let color = || -> Result<Color, String> {
            if value.len() != 7
                || !value.starts_with('#')
                || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("expected color #RRGGBB".into());
            }
            let n = u32::from_str_radix(&value[1..], 16).map_err(|_| "invalid color")?;
            Ok(Color((n >> 16) as u8, (n >> 8) as u8, n as u8))
        };
        match (group, name) {
            ("canvas", "width") => self.width = number(240, 1440)?,
            ("canvas", "padding") => self.padding = number(0, 64)?,
            ("link", "hover") => self.hover = color()?,
            ("link", "active") => self.active = color()?,
            ("card", "paper") => self.card.paper = color()?,
            ("card", "ink") => self.card.ink = color()?,
            ("card", "border") => self.card.border = color()?,
            ("card", "border-width") => self.card.border_width = number(0, 4)?,
            ("card", "radius") => self.card.radius = number(0, 32)?,
            ("card", "padding") => self.card.padding = number(0, 48)?,
            ("card", "gap") => self.card.gap = number(0, 64)?,
            ("image", "width") => self.image.width = number(16, 1440)?,
            ("image", "height") => self.image.height = number(16, 1536)?,
            ("image", "align") => self.image.align = align()?,
            ("image", "gap") => self.image.gap = number(0, 64)?,
            ("row", "gap") => self.row_gap = number(0, 64)?,
            ("row", "min-width") => self.row_min_width = number(160, 640)?,
            ("text" | "heading" | "section" | "link", property)
                if matches!(
                    property,
                    "size" | "weight" | "align" | "line-height" | "gap"
                ) =>
            {
                let target = match group {
                    "heading" => &mut self.heading,
                    "section" => &mut self.section,
                    "link" => &mut self.link,
                    _ => &mut self.text,
                };
                match property {
                    "size" => target.size = number(12, 72)?,
                    "weight" => {
                        let n = number(400, 600)?;
                        if !matches!(n, 400 | 600) {
                            return Err("weight must be 400 or 600".into());
                        }
                        target.weight = n;
                    }
                    "align" => target.align = align()?,
                    "line-height" => target.line_height = number(100, 220)?,
                    _ => target.gap = number(0, 64)?,
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    #[test]
    fn versioned_pixel_typography_and_cards_are_bounded() {
        let rich=parse("@style 0.2\ncanvas\n  width 900\n  padding 24\ntext\n  size 19\n  weight 400\n  line-height 165\n  align center\ncard\n  radius 16\n  border-width 2\nlink\n  hover #abcdef\nimage\n  align right\n").unwrap();
        assert!(rich.rich.as_ref().unwrap().valid());
        assert_eq!(rich.rich.as_ref().unwrap().text.line_height, 165);
        assert!(parse("@style 0.1\ncanvas\n  width 900").is_err());
        assert_eq!(parse("@style 0.1").unwrap(), Stylesheet::default());
        for property in [
            "size 1000",
            "weight 500",
            "align justify",
            "line-height 99",
            "gap 1000",
            "position absolute",
            "ink #oops",
        ] {
            assert!(
                parse(&format!("@style 0.2\ntext\n  {property}\n")).is_err(),
                "{property}"
            );
        }
        for property in ["radius 100", "padding 999", "border-width 8", "script run"] {
            assert!(parse(&format!("@style 0.2\ncard\n  {property}\n")).is_err());
        }
        assert!(parse("@style 0.1\ntext\n  size 18").is_err());
        assert!(parse("@style 0.2\ntext\n  size 18\n  size 20").is_err());
    }
}
