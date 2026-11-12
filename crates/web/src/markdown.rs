use ggreddit_api::media::MediaSigner;
use pulldown_cmark::{html, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd, TextMergeStream};
use serde_json::Value;
use url::Url;

use crate::{view::sanitize_html, ImageDisplay};

struct Markers {
	spoiler_open: String,
	spoiler_close: String,
	escaped_caret: String,
}

#[derive(Clone, Copy)]
pub struct Renderer<'a> {
	signer: &'a MediaSigner,
	image_display: ImageDisplay,
}

impl<'a> Renderer<'a> {
	pub const fn new(signer: &'a MediaSigner, image_display: ImageDisplay) -> Self {
		Self { signer, image_display }
	}

	pub fn render(self, source: &str, metadata: Option<&Value>) -> String {
		render(source, metadata, self.signer, self.image_display)
	}
}

pub fn render(source: &str, metadata: Option<&Value>, signer: &MediaSigner, image_display: ImageDisplay) -> String {
	let (source, markers) = preprocess(source);
	let parser = Parser::new_ext(&source, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH);
	let input: Vec<_> = TextMergeStream::new(parser).map(Event::into_static).collect();
	let events = adapt_events(&input, &markers, metadata, signer, image_display);
	let mut output = String::new();
	html::push_html(&mut output, events.into_iter());
	sanitize_html(&output, signer)
}

fn preprocess(source: &str) -> (String, Markers) {
	let mut nonce = 0_u32;
	let markers = loop {
		let marker = |name| format!("\u{e000}ggreddit-{nonce}-{name}\u{e001}");
		let markers = Markers {
			spoiler_open: marker("spoiler-open"),
			spoiler_close: marker("spoiler-close"),
			escaped_caret: marker("escaped-caret"),
		};
		if !source.contains(&markers.spoiler_open) && !source.contains(&markers.spoiler_close) && !source.contains(&markers.escaped_caret) {
			break markers;
		}
		nonce = nonce.saturating_add(1);
	};

	let mut output = String::with_capacity(source.len());
	let mut fence: Option<(u8, usize)> = None;
	for line in source.split_inclusive('\n') {
		let trimmed = line.trim_start_matches(' ');
		let indent = line.len() - trimmed.len();
		let delimiter = (indent <= 3).then(|| fence_delimiter(trimmed)).flatten();
		if let Some((character, length)) = delimiter {
			match fence {
				Some((open, minimum)) if open == character && length >= minimum => fence = None,
				None => fence = Some((character, length)),
				_ => {}
			}
			output.push_str(line);
		} else if fence.is_some() || indent >= 4 {
			output.push_str(line);
		} else if let Some(prefix_length) = reddit_heading_prefix(trimmed) {
			output.push_str(&line[..indent + prefix_length]);
			output.push(' ');
			preprocess_inline(&line[indent + prefix_length..], &markers, &mut output);
		} else {
			preprocess_inline(line, &markers, &mut output);
		}
	}
	(output, markers)
}

fn reddit_heading_prefix(line: &str) -> Option<usize> {
	let length = line.as_bytes().iter().take_while(|&&byte| byte == b'#').count();
	if !(1..=6).contains(&length) {
		return None;
	}
	line
		.as_bytes()
		.get(length)
		.is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'#')
		.then_some(length)
}

fn fence_delimiter(line: &str) -> Option<(u8, usize)> {
	let first = *line.as_bytes().first()?;
	if !matches!(first, b'`' | b'~') {
		return None;
	}
	let length = line.as_bytes().iter().take_while(|&&byte| byte == first).count();
	(length >= 3).then_some((first, length))
}

fn preprocess_inline(line: &str, markers: &Markers, output: &mut String) {
	let mut index = 0;
	let mut code_ticks = 0;
	while index < line.len() {
		if line.as_bytes()[index] == b'`' {
			let ticks = line.as_bytes()[index..].iter().take_while(|&&byte| byte == b'`').count();
			if code_ticks == 0 {
				code_ticks = ticks;
			} else if code_ticks == ticks {
				code_ticks = 0;
			}
			output.push_str(&line[index..index + ticks]);
			index += ticks;
			continue;
		}
		if code_ticks == 0 && line[index..].starts_with("\\^") {
			output.push_str(&markers.escaped_caret);
			index += 2;
			continue;
		}
		if code_ticks == 0 && line[index..].starts_with(">!") && !is_escaped(line, index) {
			if let Some(close) = find_spoiler_close(line, index + 2) {
				output.push_str(&markers.spoiler_open);
				output.push_str(&line[index + 2..close]);
				output.push_str(&markers.spoiler_close);
				index = close + 2;
				continue;
			}
			if line[..index].chars().all(char::is_whitespace) {
				output.push_str("\\>!");
				index += 2;
				continue;
			}
		}
		let character = line[index..].chars().next().expect("index remains on a character boundary");
		output.push(character);
		index += character.len_utf8();
	}
}

fn find_spoiler_close(line: &str, mut index: usize) -> Option<usize> {
	let mut code_ticks = 0;
	while index < line.len() {
		if line.as_bytes()[index] == b'`' {
			let ticks = line.as_bytes()[index..].iter().take_while(|&&byte| byte == b'`').count();
			if code_ticks == 0 {
				code_ticks = ticks;
			} else if code_ticks == ticks {
				code_ticks = 0;
			}
			index += ticks;
			continue;
		}
		if code_ticks == 0 && line[index..].starts_with("!<") && !is_escaped(line, index) {
			return Some(index);
		}
		index += line[index..].chars().next()?.len_utf8();
	}
	None
}

fn is_escaped(source: &str, index: usize) -> bool {
	source[..index].bytes().rev().take_while(|&byte| byte == b'\\').count() % 2 == 1
}

fn adapt_events(input: &[Event<'static>], markers: &Markers, metadata: Option<&Value>, signer: &MediaSigner, image_display: ImageDisplay) -> Vec<Event<'static>> {
	let mut output = Vec::with_capacity(input.len());
	let mut index = 0;
	let mut link_depth = 0_u32;
	while index < input.len() {
		match &input[index] {
			Event::Start(Tag::Image { dest_url, .. }) => {
				let end = input[index + 1..]
					.iter()
					.position(|event| matches!(event, Event::End(TagEnd::Image)))
					.map_or(input.len(), |offset| index + offset + 1);
				let alt = image_alt(&input[index + 1..end]);
				append_image(&mut output, dest_url, &alt, metadata, signer, image_display);
				index = end.saturating_add(1);
				continue;
			}
			Event::Start(Tag::Link { .. }) => {
				link_depth = link_depth.saturating_add(1);
				output.push(input[index].clone());
			}
			Event::End(TagEnd::Link) => {
				link_depth = link_depth.saturating_sub(1);
				output.push(input[index].clone());
			}
			Event::Text(text) => append_text(&mut output, text, markers, link_depth == 0),
			Event::Html(html) | Event::InlineHtml(html) => output.push(Event::Text(html.clone())),
			_ => output.push(input[index].clone()),
		}
		index += 1;
	}
	output
}

fn image_alt(events: &[Event<'_>]) -> String {
	let mut alt = String::new();
	for event in events {
		match event {
			Event::Text(text) | Event::Code(text) => alt.push_str(text),
			Event::SoftBreak | Event::HardBreak => alt.push(' '),
			_ => {}
		}
	}
	alt
}

fn append_image(output: &mut Vec<Event<'static>>, destination: &str, alt: &str, metadata: Option<&Value>, signer: &MediaSigner, display: ImageDisplay) {
	let Some(source) = resolve_media_source(destination, metadata) else {
		output.push(Event::Text(fallback_alt(alt).into()));
		return;
	};
	let Some(url) = signer.signed_media_url(&source) else {
		output.push(Event::Text(fallback_alt(alt).into()));
		return;
	};
	match display {
		ImageDisplay::Inline => {
			output.push(Event::Start(Tag::Image {
				link_type: LinkType::Inline,
				dest_url: url.into(),
				title: CowStr::Borrowed(""),
				id: CowStr::Borrowed(""),
			}));
			output.push(Event::Text(fallback_alt(alt).into()));
			output.push(Event::End(TagEnd::Image));
		}
		ImageDisplay::Link => {
			output.push(link_start(url));
			output.push(Event::Text(image_link_label(alt, &source).into()));
			output.push(Event::End(TagEnd::Link));
		}
	}
}

fn resolve_media_source(destination: &str, metadata: Option<&Value>) -> Option<String> {
	if Url::parse(&destination.replace("&amp;", "&")).is_ok() || destination.starts_with("//") {
		return Some(destination.to_owned());
	}
	let metadata_source = metadata
		.and_then(Value::as_object)
		.and_then(|metadata| metadata.get(destination).or_else(|| destination.strip_prefix("giphy|").and_then(|id| metadata.get(id))))
		.and_then(|media| {
			media.pointer("/s/gif").or_else(|| media.pointer("/s/u")).or_else(|| {
				media
					.get("p")
					.and_then(Value::as_array)
					.and_then(|previews| previews.last())
					.and_then(|preview| preview.get("u"))
			})
		})
		.and_then(Value::as_str)
		.map(str::to_owned);
	metadata_source.or_else(|| giphy_source(destination))
}

fn giphy_source(destination: &str) -> Option<String> {
	let id = destination.strip_prefix("giphy|")?;
	if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
		return None;
	}
	Some(format!("https://media.giphy.com/media/{id}/giphy.gif"))
}

fn fallback_alt(alt: &str) -> String {
	if alt.trim().is_empty() {
		"media".into()
	} else {
		alt.to_owned()
	}
}

fn image_link_label(alt: &str, source: &str) -> String {
	let kind = if alt.eq_ignore_ascii_case("gif") { "GIF" } else { "image" };
	Url::parse(&source.replace("&amp;", "&"))
		.ok()
		.and_then(|url| url.host_str().map(str::to_owned))
		.map_or_else(|| format!("View {kind}"), |host| format!("View {kind} from {host}"))
}

fn append_text(output: &mut Vec<Event<'static>>, text: &str, markers: &Markers, autolink: bool) {
	let mut plain = String::new();
	let mut index = 0;
	while index < text.len() {
		if text[index..].starts_with(&markers.spoiler_open) {
			flush_text(output, &mut plain);
			output.push(Event::InlineHtml(CowStr::Borrowed(r#"<span class="md-spoiler-text" tabindex="0">"#)));
			index += markers.spoiler_open.len();
			continue;
		}
		if text[index..].starts_with(&markers.spoiler_close) {
			flush_text(output, &mut plain);
			output.push(Event::InlineHtml(CowStr::Borrowed("</span>")));
			index += markers.spoiler_close.len();
			continue;
		}
		if text[index..].starts_with(&markers.escaped_caret) {
			plain.push('^');
			index += markers.escaped_caret.len();
			continue;
		}
		if text[index..].starts_with("^(") {
			if let Some(length) = text[index + 2..].find(')') {
				flush_text(output, &mut plain);
				output.push(Event::InlineHtml(CowStr::Borrowed("<sup>")));
				output.push(Event::Text(text[index + 2..index + 2 + length].to_owned().into()));
				output.push(Event::InlineHtml(CowStr::Borrowed("</sup>")));
				index += length + 3;
				continue;
			}
		}
		if text[index..].starts_with('^') {
			let length = text[index + 1..].find(char::is_whitespace).unwrap_or(text.len() - index - 1);
			if length > 0 {
				flush_text(output, &mut plain);
				output.push(Event::InlineHtml(CowStr::Borrowed("<sup>")));
				output.push(Event::Text(text[index + 1..index + 1 + length].to_owned().into()));
				output.push(Event::InlineHtml(CowStr::Borrowed("</sup>")));
				index += length + 1;
				continue;
			}
		}
		if autolink && ref_boundary(text, index) {
			if let Some((length, destination)) = reddit_reference(&text[index..]) {
				flush_text(output, &mut plain);
				let label = &text[index..index + length];
				output.push(link_start(destination));
				output.push(Event::Text(label.to_owned().into()));
				output.push(Event::End(TagEnd::Link));
				index += length;
				continue;
			}
		}
		if autolink && url_boundary(text, index) && (text[index..].starts_with("https://") || text[index..].starts_with("http://")) {
			let length = url_length(&text[index..]);
			if length > 0 {
				flush_text(output, &mut plain);
				let url = &text[index..index + length];
				output.push(link_start(url.to_owned()));
				output.push(Event::Text(url.to_owned().into()));
				output.push(Event::End(TagEnd::Link));
				index += length;
				continue;
			}
		}
		let character = text[index..].chars().next().expect("index remains on a character boundary");
		plain.push(character);
		index += character.len_utf8();
	}
	flush_text(output, &mut plain);
}

fn flush_text(output: &mut Vec<Event<'static>>, plain: &mut String) {
	if !plain.is_empty() {
		output.push(Event::Text(std::mem::take(plain).into()));
	}
}

fn link_start(destination: String) -> Event<'static> {
	Event::Start(Tag::Link {
		link_type: LinkType::Autolink,
		dest_url: destination.into(),
		title: CowStr::Borrowed(""),
		id: CowStr::Borrowed(""),
	})
}

fn ref_boundary(text: &str, index: usize) -> bool {
	index == 0
		|| text[..index]
			.chars()
			.next_back()
			.is_some_and(|character| character.is_whitespace() || matches!(character, '(' | '[' | '{' | '>' | '—'))
}

fn url_boundary(text: &str, index: usize) -> bool {
	index == 0
		|| !text[..index]
			.chars()
			.next_back()
			.is_some_and(|character| character.is_alphanumeric() || matches!(character, '_' | '-'))
}

fn reddit_reference(text: &str) -> Option<(usize, String)> {
	let (prefix, route, user) = if text.starts_with("/r/") {
		("/r/", "/r/", false)
	} else if text.starts_with("r/") {
		("r/", "/r/", false)
	} else if text.starts_with("/u/") {
		("/u/", "/user/", true)
	} else if text.starts_with("u/") {
		("u/", "/user/", true)
	} else {
		return None;
	};
	let name_length = text[prefix.len()..]
		.chars()
		.take_while(|character| character.is_ascii_alphanumeric() || *character == '_' || (user && *character == '-'))
		.map(char::len_utf8)
		.sum::<usize>();
	if name_length == 0 {
		return None;
	}
	let name = &text[prefix.len()..prefix.len() + name_length];
	Some((prefix.len() + name_length, format!("{route}{name}")))
}

fn url_length(text: &str) -> usize {
	let mut length = text
		.char_indices()
		.take_while(|(_, character)| !character.is_whitespace() && !matches!(character, '<' | '>' | '"' | '\''))
		.map(|(index, character)| index + character.len_utf8())
		.last()
		.unwrap_or(0);
	while length > 0 {
		let candidate = &text[..length];
		let last = candidate.chars().next_back().expect("candidate is not empty");
		let trim = matches!(last, '.' | ',' | '!' | '?' | ';' | ':')
			|| (last == ')' && candidate.matches(')').count() > candidate.matches('(').count())
			|| (last == ']' && candidate.matches(']').count() > candidate.matches('[').count())
			|| (last == '}' && candidate.matches('}').count() > candidate.matches('{').count());
		if !trim {
			break;
		}
		length -= last.len_utf8();
	}
	length
}
