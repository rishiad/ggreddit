use super::{SelectChoice, SortControls};
use neddit_api::service::ListingTime;

pub fn feed_controls(action: &str, active_sort: &'static str, active_time: &str, include_controversial: bool) -> SortControls {
	let mut sorts = vec![("hot", "Hot"), ("new", "New"), ("rising", "Rising"), ("top", "Top")];
	if include_controversial {
		sorts.push(("controversial", "Controversial"));
	}
	SortControls {
		action: action.into(),
		label: "Post sorting",
		choice_label: "Sort posts",
		sorts: select_choices(sorts, active_sort),
		times: if active_sort == "top" { listing_time_choices(active_time) } else { Vec::new() },
	}
}

pub fn comment_sort_controls(active: &'static str, permalink: &str) -> SortControls {
	SortControls {
		action: permalink.into(),
		label: "Comment sorting",
		choice_label: "Sort comments",
		sorts: select_choices(
			[("best", "Best"), ("top", "Top"), ("new", "New"), ("old", "Old"), ("controversial", "Controversial")],
			active,
		),
		times: Vec::new(),
	}
}

pub fn user_controls(action: &str, active_sort: &'static str, active_time: &str) -> SortControls {
	SortControls {
		action: action.into(),
		label: "User activity sorting",
		choice_label: "Sort user activity",
		sorts: select_choices([("new", "New"), ("hot", "Hot"), ("top", "Top"), ("controversial", "Controversial")], active_sort),
		times: if active_sort == "top" { listing_time_choices(active_time) } else { Vec::new() },
	}
}

fn select_choices(values: impl IntoIterator<Item = (&'static str, &'static str)>, active: &str) -> Vec<SelectChoice> {
	values
		.into_iter()
		.map(|(value, label)| SelectChoice {
			value,
			label,
			checked: value == active,
		})
		.collect()
}

pub(crate) fn listing_time_choices(active: &str) -> Vec<SelectChoice> {
	select_choices(
		[
			(ListingTime::Hour.as_str(), "Past hour"),
			(ListingTime::Day.as_str(), "Past 24 hours"),
			(ListingTime::Week.as_str(), "Past week"),
			(ListingTime::Month.as_str(), "Past month"),
			(ListingTime::Year.as_str(), "Past year"),
			(ListingTime::All.as_str(), "All time"),
		],
		active,
	)
}
