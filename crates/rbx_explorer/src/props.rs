//! The Properties sheet's content: an instance's stored properties grouped
//! by the API dump's categories, as Studio groups them, each value written
//! out the way Studio's panel shows it, plus its attributes and tags.

use std::collections::BTreeMap;

use rbx_dom::{BrickColor, CFrameData, Color3Data, Content, Ref, UDim, Variant, WeakDom};
use rbx_reflection::ReflectionDatabase;

/// How a value is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    /// Numbers and vectors, set in the code font.
    Mono(String),
    Bool(bool),
    /// A colour swatch and its description.
    Color([u8; 3], String),
    /// A reference to another instance: its name, or nothing.
    Ref(Option<Ref>, String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub title: String,
    pub properties: Vec<Property>,
}

/// Studio's own category order; anything else follows alphabetically.
const CATEGORY_ORDER: &[&str] = &[
    "Data",
    "Appearance",
    "Text",
    "Image",
    "Behavior",
    "Part",
    "Collision",
    "Transform",
    "Pivot",
    "Surface",
    "Surface Inputs",
    "Shape",
    "Assembly",
    "Character",
    "Controls",
    "Camera",
    "Physics",
    "Sound",
    "Emission",
    "Motion",
    "Particles",
    "Scripting",
    "Localization",
    "State",
    "Size",
    "Position",
    "Layout",
    "Rendering",
];

/// Properties a file stores that are not worth a row: the script text (it
/// has its own screen) and the raw blobs shown below as attributes/tags.
const SKIPPED: &[&str] = &[
    "Source",
    "Tags",
    "AttributesSerialize",
    "Attributes",
    "ScriptGuid",
];

/// Russian titles for the categories.
fn category_title(category: &str) -> String {
    match category {
        "Data" => "Данные",
        "Appearance" => "Внешний вид",
        "Text" => "Текст",
        "Image" => "Изображение",
        "Behavior" => "Поведение",
        "Part" => "Деталь",
        "Collision" => "Столкновения",
        "Transform" => "Трансформация",
        "Pivot" => "Опорная точка",
        "Surface" => "Поверхность",
        "Shape" => "Форма",
        "Assembly" => "Сборка",
        "Character" => "Персонаж",
        "Controls" => "Управление",
        "Camera" => "Камера",
        "Physics" => "Физика",
        "Sound" => "Звук",
        "Emission" => "Излучение",
        "Motion" => "Движение",
        "Particles" => "Частицы",
        "Scripting" => "Скрипт",
        "Localization" => "Локализация",
        "State" => "Состояние",
        "Layout" => "Разметка",
        "Rendering" => "Отрисовка",
        "Size" => "Размер",
        "Position" => "Позиция",
        "Other" => "Прочее",
        other => other,
    }
    .to_string()
}

/// Every group of `referent`'s sheet, in order.
pub fn sheet(dom: &WeakDom, referent: Ref) -> Vec<Group> {
    let Some(instance) = dom.get(referent) else {
        return Vec::new();
    };
    let database = ReflectionDatabase::shared();
    let class = instance.class();
    let mut groups: BTreeMap<(usize, String), Vec<Property>> = BTreeMap::new();
    let rank = |category: &str| {
        CATEGORY_ORDER
            .iter()
            .position(|known| *known == category)
            .unwrap_or(CATEGORY_ORDER.len())
    };

    let data = (rank("Data"), "Data".to_string());
    groups.entry(data.clone()).or_default().extend([
        Property {
            name: "Name".into(),
            value: Value::Text(instance.name().to_string()),
        },
        Property {
            name: "ClassName".into(),
            value: Value::Text(class.to_string()),
        },
    ]);
    if let Some(source) = instance.properties().get("Source") {
        let text = match source {
            Variant::String(text) => text.clone(),
            Variant::Unknown { raw, .. } => String::from_utf8_lossy(raw).into_owned(),
            _ => String::new(),
        };
        let lines = crate::textops::line_count(&text);
        groups.entry(data.clone()).or_default().push(Property {
            name: "Source".into(),
            value: Value::Text(format!(
                "{} · {}",
                plural(lines, "строка", "строки", "строк"),
                human_bytes(text.len())
            )),
        });
    }

    for (key, value) in instance.properties() {
        if SKIPPED.contains(&key.as_str()) {
            continue;
        }
        let name = database.canonical_name(class, key);
        if name == "Name" {
            continue;
        }
        let descriptor = database.resolve_property(class, name);
        let category = match descriptor {
            Some(descriptor) if !descriptor.is_hidden() => descriptor.category.as_str(),
            _ => "Other",
        };
        let enum_type = descriptor.map(|descriptor| descriptor.value_type.as_str());
        let key_rank = if category == "Other" {
            usize::MAX
        } else {
            rank(category)
        };
        groups
            .entry((key_rank, category.to_string()))
            .or_default()
            .push(Property {
                name: name.to_string(),
                value: describe(dom, value, enum_type),
            });
    }

    let mut out: Vec<Group> = groups
        .into_iter()
        .map(|((_, category), mut properties)| {
            if category != "Data" {
                properties.sort_by(|a, b| a.name.cmp(&b.name));
            }
            Group {
                title: category_title(&category),
                properties,
            }
        })
        .collect();

    let attributes = instance.attributes();
    if !attributes.is_empty() {
        out.push(Group {
            title: "Атрибуты".into(),
            properties: attributes
                .iter()
                .map(|(name, value)| Property {
                    name: name.clone(),
                    value: describe(dom, value, None),
                })
                .collect(),
        });
    }
    let tags = instance.tags();
    if !tags.is_empty() {
        out.push(Group {
            title: "Теги".into(),
            properties: vec![Property {
                name: plural(tags.len(), "тег", "тега", "тегов"),
                value: Value::Text(tags.join(", ")),
            }],
        });
    }
    out
}

/// `count` with the Russian noun form that goes with it.
pub fn plural(count: usize, one: &str, few: &str, many: &str) -> String {
    let form = match (count % 10, count % 100) {
        (_, 11..=14) => many,
        (1, _) => one,
        (2..=4, _) => few,
        _ => many,
    };
    format!("{} {form}", group_digits(count))
}

/// `1234567` as `1 234 567` (with narrow no-break spaces).
pub fn group_digits(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(digit);
    }
    out
}

/// A byte count as Russian units, `4,2 МБ`.
pub fn human_bytes(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["Б", "КБ", "МБ", "ГБ"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else if value < 10.0 {
        format!("{:.1} {}", value, UNITS[unit]).replace('.', ",")
    } else {
        format!("{:.0} {}", value, UNITS[unit])
    }
}

/// A float as Studio prints it: up to three decimals, no trailing zeros.
pub fn number(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "nan".into()
        } else if value > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let mut text = format!("{value:.3}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        text = "0".into();
    }
    text
}

fn f(value: f32) -> String {
    number(f64::from(value))
}

fn color8(color: &Color3Data) -> [u8; 3] {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    [channel(color.r), channel(color.g), channel(color.b)]
}

fn udim(value: &UDim) -> String {
    format!("{}, {}", f(value.scale), value.offset)
}

/// Roblox's `CFrame:ToOrientation()` (Y, X, Z order), in degrees.
pub fn orientation(cframe: &CFrameData) -> [f64; 3] {
    let m = cframe.rotation.map(f64::from);
    let x = (-m[5]).clamp(-1.0, 1.0).asin();
    let (y, z) = if m[5].abs() < 0.999_999 {
        (m[2].atan2(m[8]), m[3].atan2(m[4]))
    } else {
        ((-m[6]).atan2(m[0]), 0.0)
    };
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

fn cframe(value: &CFrameData) -> String {
    let p = &value.position;
    let [x, y, z] = orientation(value);
    format!(
        "{}, {}, {} · {}°, {}°, {}°",
        f(p.x),
        f(p.y),
        f(p.z),
        number(x),
        number(y),
        number(z)
    )
}

fn name_of(dom: &WeakDom, referent: Ref) -> Option<String> {
    dom.get(referent)
        .map(|instance| instance.name().to_string())
}

/// How `value` reads in the sheet. `value_type` is the property's type in
/// the API dump, which names the enum an `Enum` value belongs to.
pub fn describe(dom: &WeakDom, value: &Variant, value_type: Option<&str>) -> Value {
    match value {
        Variant::String(text) => {
            let mut shown: String = text.chars().take(400).collect();
            if shown.len() < text.len() {
                shown.push('…');
            }
            if shown.is_empty() {
                Value::Text("—".into())
            } else {
                Value::Text(shown)
            }
        }
        Variant::Bool(on) => Value::Bool(*on),
        Variant::Int32(n) => Value::Mono(n.to_string()),
        Variant::Int64(n) => Value::Mono(n.to_string()),
        Variant::Float32(n) => Value::Mono(f(*n)),
        Variant::Float64(n) => Value::Mono(number(*n)),
        Variant::BrickColor(n) => match BrickColor::from_number(*n) {
            Some(color) => Value::Color(color.rgb, color.name.to_string()),
            None => Value::Mono(n.to_string()),
        },
        Variant::Color3(color) => {
            let rgb = color8(color);
            Value::Color(rgb, format!("{}, {}, {}", rgb[0], rgb[1], rgb[2]))
        }
        Variant::Color3uint8 { r, g, b } => Value::Color([*r, *g, *b], format!("{r}, {g}, {b}")),
        Variant::Vector2(v) => Value::Mono(format!("{}, {}", f(v.x), f(v.y))),
        Variant::Vector3(v) => Value::Mono(format!("{}, {}, {}", f(v.x), f(v.y), f(v.z))),
        Variant::Vector3int16 { x, y, z } => Value::Mono(format!("{x}, {y}, {z}")),
        Variant::Ray { origin, direction } => Value::Mono(format!(
            "{}, {}, {} → {}, {}, {}",
            f(origin.x),
            f(origin.y),
            f(origin.z),
            f(direction.x),
            f(direction.y),
            f(direction.z)
        )),
        Variant::Faces(faces) => {
            let names: Vec<&str> = [
                (faces.right, "Right"),
                (faces.top, "Top"),
                (faces.back, "Back"),
                (faces.left, "Left"),
                (faces.bottom, "Bottom"),
                (faces.front, "Front"),
            ]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, name)| *name)
            .collect();
            Value::Text(if names.is_empty() {
                "—".into()
            } else {
                names.join(", ")
            })
        }
        Variant::Axes(axes) => {
            let names: Vec<&str> = [(axes.x, "X"), (axes.y, "Y"), (axes.z, "Z")]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, name)| *name)
                .collect();
            Value::Text(if names.is_empty() {
                "—".into()
            } else {
                names.join(", ")
            })
        }
        Variant::CFrame(value) => Value::Mono(cframe(value)),
        Variant::OptionalCFrame(value) => match value {
            Some(value) => Value::Mono(cframe(value)),
            None => Value::Text("—".into()),
        },
        Variant::Enum(n) => {
            let name = value_type
                .and_then(|enum_type| ReflectionDatabase::shared().enum_name(enum_type, *n));
            match name {
                Some(name) => Value::Text(name.to_string()),
                None => Value::Mono(n.to_string()),
            }
        }
        Variant::Ref(referent) => match name_of(dom, *referent) {
            Some(name) => Value::Ref(Some(*referent), name),
            None => Value::Ref(None, "—".into()),
        },
        Variant::NumberSequence(sequence) => {
            let points: Vec<String> = sequence
                .keypoints
                .iter()
                .take(6)
                .map(|point| format!("{}:{}", f(point.time), f(point.value)))
                .collect();
            Value::Mono(points.join("  "))
        }
        Variant::ColorSequence(sequence) => match sequence.keypoints.first() {
            Some(first) => {
                let rgb = color8(&first.color);
                let last = sequence
                    .keypoints
                    .last()
                    .map(|point| color8(&point.color))
                    .unwrap_or(rgb);
                Value::Color(
                    rgb,
                    format!(
                        "{} → {} · {}",
                        hex(rgb),
                        hex(last),
                        plural(sequence.keypoints.len(), "точка", "точки", "точек")
                    ),
                )
            }
            None => Value::Text("—".into()),
        },
        Variant::NumberRange(range) => Value::Mono(format!("{} – {}", f(range.min), f(range.max))),
        Variant::Rect(rect) => Value::Mono(format!(
            "{}, {}, {}, {}",
            f(rect.min.x),
            f(rect.min.y),
            f(rect.max.x),
            f(rect.max.y)
        )),
        Variant::PhysicalProperties(properties) => match properties {
            rbx_dom::PhysicalProperties::Default => Value::Text("По умолчанию".into()),
            rbx_dom::PhysicalProperties::Custom {
                density,
                friction,
                elasticity,
                ..
            } => Value::Mono(format!(
                "ρ {} · μ {} · e {}",
                f(*density),
                f(*friction),
                f(*elasticity)
            )),
        },
        Variant::SharedString(_) => Value::Text("общие данные".into()),
        Variant::UDim(value) => Value::Mono(format!("{{{}}}", udim(value))),
        Variant::UDim2(value) => {
            Value::Mono(format!("{{{}}}, {{{}}}", udim(&value.x), udim(&value.y)))
        }
        Variant::UniqueId(id) => {
            Value::Mono(format!("{:016x}{:08x}{:08x}", id.random, id.time, id.index))
        }
        Variant::Font(font) => {
            let family = font
                .family
                .rsplit('/')
                .next()
                .unwrap_or(&font.family)
                .trim_end_matches(".json");
            Value::Text(format!("{family} · {}", font.weight))
        }
        Variant::SecurityCapabilities(bits) => Value::Mono(format!("0x{bits:x}")),
        Variant::Content(content) => match content {
            Content::None => Value::Text("—".into()),
            Content::Uri(uri) => Value::Text(uri.clone()),
            Content::Object(referent) => match name_of(dom, *referent) {
                Some(name) => Value::Ref(Some(*referent), name),
                None => Value::Ref(None, "—".into()),
            },
        },
        Variant::Unknown { raw, .. } => {
            Value::Text(format!("двоичные данные, {}", human_bytes(raw.len())))
        }
    }
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom::Vector3Data;

    #[test]
    fn numbers_and_plurals() {
        assert_eq!(number(1.0), "1");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(-0.0001), "0");
        assert_eq!(number(1.23456), "1.235");
        assert_eq!(plural(1, "строка", "строки", "строк"), "1 строка");
        assert_eq!(plural(3, "строка", "строки", "строк"), "3 строки");
        assert_eq!(plural(11, "строка", "строки", "строк"), "11 строк");
        assert_eq!(plural(21, "строка", "строки", "строк"), "21 строка");
        assert_eq!(group_digits(2418), "2\u{202f}418");
        assert_eq!(human_bytes(512), "512 Б");
        assert_eq!(human_bytes(4_404_019), "4,2 МБ");
        assert_eq!(human_bytes(122_880), "120 КБ");
    }

    #[test]
    fn orientation_matches_roblox() {
        // CFrame.Angles(0, math.rad(90), 0)
        let cframe = CFrameData {
            position: Vector3Data {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            rotation: [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0],
        };
        let [x, y, z] = orientation(&cframe);
        assert!((x.abs() + (y - 90.0).abs() + z.abs()) < 1e-6);
    }

    #[test]
    fn a_part_sheet_groups_its_properties() {
        let mut dom = WeakDom::new();
        let part = dom.new_instance("Part", "Baseplate", None);
        let instance = dom.get_mut(part).unwrap();
        instance
            .properties_mut()
            .insert("Anchored".into(), Variant::Bool(true));
        instance.properties_mut().insert(
            "Color3uint8".into(),
            Variant::Color3uint8 {
                r: 91,
                g: 154,
                b: 76,
            },
        );
        instance
            .properties_mut()
            .insert("Material".into(), Variant::Enum(512));
        let groups = sheet(&dom, part);
        assert_eq!(groups[0].title, "Данные");
        let all: Vec<&Property> = groups.iter().flat_map(|g| &g.properties).collect();
        let find = |name: &str| all.iter().find(|p| p.name == name).unwrap().value.clone();
        assert_eq!(find("Anchored"), Value::Bool(true));
        assert_eq!(
            find("Color"),
            Value::Color([91, 154, 76], "91, 154, 76".into())
        );
        assert_eq!(find("Material"), Value::Text("Wood".into()));
    }
}
