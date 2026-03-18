use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let auth_path = dirs::config_dir().unwrap().join("tui-x").join("auth.json");
    let store: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&auth_path)?)?;
    let account = &store["accounts"]["main"];
    let auth_token = account["auth_token"].as_str().unwrap();
    let ct0 = account["ct0"].as_str().unwrap();

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36")
        .build()?;

    // Use cached ops
    let cache_path = dirs::cache_dir().unwrap().join("tui-x").join("graphql_ops.json");
    let ops: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cache_path)?)?;
    let qid = ops["operations"]["HomeLatestTimeline"]["query_id"].as_str().unwrap();

    let variables = serde_json::json!({"count": 1, "includePromotedContent": false, "latestControlAvailable": true, "requestContext": "launch"});
    let features = serde_json::json!({"rweb_tipjar_consumption_enabled":true,"responsive_web_graphql_exclude_directive_enabled":true,"verified_phone_label_enabled":false,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_graphql_skip_user_profile_image_extensions_enabled":false,"communities_web_enable_tweet_community_results_fetch":true,"c9s_tweet_anatomy_moderator_badge_enabled":true,"creator_subscriptions_tweet_preview_api_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"tweet_awards_web_tipping_enabled":false,"creator_subscriptions_quote_tweet_preview_enabled":false,"freedom_of_speech_not_reach_fetch_enabled":true,"standardized_nudges_misinfo":true,"tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled":true,"rweb_video_timestamps_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_enhance_cards_enabled":false});

    let bearer = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";
    let url = format!("https://x.com/i/api/graphql/{qid}/HomeLatestTimeline");

    let body: serde_json::Value = client
        .get(&url)
        .header("authorization", format!("Bearer {bearer}"))
        .header("x-csrf-token", ct0)
        .header("x-twitter-auth-type", "OAuth2Session")
        .header("x-twitter-active-user", "yes")
        .header("cookie", format!("auth_token={auth_token}; ct0={ct0}"))
        .header("referer", "https://x.com/")
        .query(&[("variables", serde_json::to_string(&variables)?), ("features", serde_json::to_string(&features)?)])
        .send().await?.json().await?;

    // Find first tweet result and dump its structure
    if let Some(entries) = body.pointer("/data/home/home_timeline_urt/instructions")
        .and_then(|i| i.as_array())
        .and_then(|insts| insts.iter().find(|i| i.get("type").and_then(|t| t.as_str()) == Some("TimelineAddEntries")))
        .and_then(|i| i.get("entries"))
        .and_then(|e| e.as_array())
    {
        for entry in entries.iter().take(2) {
            if let Some(result) = entry.pointer("/content/itemContent/tweet_results/result") {
                // Show the core/user_results structure
                println!("=== __typename: {:?}", result.get("__typename"));
                println!("=== core keys: {:?}", result.get("core").map(|c| c.as_object().map(|o| o.keys().collect::<Vec<_>>())));
                if let Some(user) = result.pointer("/core/user_results/result") {
                    println!("=== user __typename: {:?}", user.get("__typename"));
                    println!("=== user rest_id: {:?}", user.get("rest_id"));
                    println!("=== user has legacy: {}", user.get("legacy").is_some());
                    if let Some(legacy) = user.get("legacy") {
                        println!("=== user legacy keys: {:?}", legacy.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                        println!("=== screen_name: {:?}", legacy.get("screen_name"));
                        println!("=== name: {:?}", legacy.get("name"));
                    }
                }
                println!();
            }
        }
    }

    Ok(())
}
