use crate::models::{CommentChild, CommentReplies, Listing, PostComments, PublicThing};

pub(super) fn clear_listing_modhash<T>(listing: &mut Listing<T>) {
	listing.data.modhash = Some(String::new());
}

pub(super) fn clear_post_comments_modhash(comments: &mut PostComments) {
	clear_listing_modhash(&mut comments.0);
	clear_comment_listing_modhash(&mut comments.1);
}

pub(super) fn clear_public_listing_modhash(listing: &mut Listing<PublicThing>) {
	clear_listing_modhash(listing);
	for child in &mut listing.data.children {
		if let PublicThing::Comment(comment) = child {
			if let CommentReplies::Listing(replies) = &mut comment.data.replies {
				clear_comment_listing_modhash(replies);
			}
		}
	}
}

fn clear_comment_listing_modhash(listing: &mut Listing<CommentChild>) {
	clear_listing_modhash(listing);
	clear_comment_children_modhash(&mut listing.data.children);
}

fn clear_comment_children_modhash(children: &mut [CommentChild]) {
	for child in children {
		if let CommentChild::Comment(comment) = child {
			if let CommentReplies::Listing(replies) = &mut comment.data.replies {
				clear_comment_listing_modhash(replies);
			}
		}
	}
}
