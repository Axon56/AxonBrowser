use anyhow::Result;

use crate::{
    browser_options, chrome,
    cli::{
        BrowserOptionChoiceArg, BrowserOptionPromptArg, BrowserScrollTargetArg, ChromeCommands,
        ChromePageCommand, ChromePageScrollDirectionArg, ChromePageStateArg, ChromeResizePresetArg,
        ChromeTabsCommand, ChromeWaitTarget, Cli, Commands, EdgeCommands, FirefoxCommands,
    },
    edge, firefox, install, render,
};

/// Run the parsed CLI command after runtime bootstrap has completed.
pub async fn run(cli: Cli) -> Result<()> {
    let firefox_flavor = current_firefox_flavor();

    match cli.command {
        Commands::InstallDeps => install::install_deps()?,
        Commands::Edge(args) => match args.command {
            EdgeCommands::Launch(inner) => {
                let summary = edge::launch::launch(
                    inner.url.as_deref(),
                    inner.profile.as_deref(),
                    inner.timeout_ms,
                    inner.poll_ms,
                )
                .await?;
                println!("{}", summary);
            }
            EdgeCommands::Attach(inner) => {
                let summary = edge::attach::attach(inner.json, inner.window_id.as_deref()).await?;
                print!("{}", summary);
            }
            EdgeCommands::Windows => {
                let windows = edge::window::list_edge_windows(None)?;
                for window in windows {
                    println!(
                        "{} {} ({}x{} at {},{})",
                        window.id, window.name, window.width, window.height, window.x, window.y
                    );
                }
            }
            EdgeCommands::Locate(inner) => {
                let node = edge::actions::click::resolve_locator(&inner.locator).await?;
                let locator = chrome::locators::ChromeLocator::parse(&inner.locator)?;
                print!(
                    "{}",
                    render::render_chrome_locator(locator.canonical_name(), &node)
                );
            }
            EdgeCommands::Screenshot(inner) => {
                let mode = if inner.active {
                    edge::screenshot::ScreenshotMode::Active
                } else {
                    edge::screenshot::ScreenshotMode::Window
                };
                let summary =
                    edge::screenshot::capture_mode(&inner.output, inner.query.as_deref(), mode)
                        .await?;
                println!("{}", summary);
            }
            EdgeCommands::Resize(inner) => {
                let summary = edge::resize::resize(
                    inner.query.as_deref(),
                    inner.preset.map(map_resize_preset),
                    inner.width,
                    inner.height,
                )?;
                println!("{}", summary);
            }
            EdgeCommands::Focus(inner) => {
                let summary = edge::actions::focus(&inner.locator).await?;
                println!("{}", summary);
            }
            EdgeCommands::Click(inner) => {
                let summary = edge::actions::click(&inner.locator).await?;
                println!("{}", summary);
            }
            EdgeCommands::Type(inner) => {
                let summary = edge::actions::type_text(&inner.locator, &inner.text).await?;
                println!("{}", summary);
            }
            EdgeCommands::Key(inner) => {
                let summary = edge::actions::press_key(&inner.locator, &inner.key).await?;
                println!("{}", summary);
            }
            EdgeCommands::PressEnter(inner) => {
                let summary = edge::actions::press_enter(inner.locator.as_deref()).await?;
                println!("{}", summary);
            }
            EdgeCommands::Hold(inner) => {
                let summary =
                    edge::actions::hold(&inner.key, inner.duration_ms, inner.locator.as_deref())
                        .await?;
                println!("{}", summary);
            }
            EdgeCommands::Scroll(inner) => {
                let scope = edge::page::root::PageScope::default();
                let summary = edge::page::actions::scroll_window(
                    &scope,
                    map_page_scroll_direction(inner.direction),
                    inner.amount,
                )
                .await?;
                println!("{}", summary);
            }
            EdgeCommands::ScrollTo(inner) => {
                let window = edge::window::find_edge_window(None)?;
                let activation_note = edge::actions::context::activate_window_note(&window.id);
                crate::window::send_key(&window.id, map_scroll_to_key(inner.target))?;
                println!(
                    "scrolled page window {} to {:?} ({})",
                    window.id, inner.target, activation_note
                );
            }
            EdgeCommands::Wait(inner) => {
                let summary = match inner.target {
                    ChromeWaitTarget::Locator(wait) => {
                        edge::wait::wait_for_locator(&wait.locator, wait.timeout_ms, wait.poll_ms)
                            .await?
                    }
                    ChromeWaitTarget::TitleChange(wait) => {
                        edge::wait::wait_for_title_change(
                            wait.from.as_deref(),
                            wait.timeout_ms,
                            wait.poll_ms,
                        )
                        .await?
                    }
                    ChromeWaitTarget::UrlChange(wait) => {
                        edge::wait::wait_for_url_change(
                            wait.from.as_deref(),
                            wait.timeout_ms,
                            wait.poll_ms,
                        )
                        .await?
                    }
                };
                println!("{}", summary);
            }
            EdgeCommands::Goto(inner) => {
                let summary = edge::goto::navigate(
                    &inner.url,
                    inner.new_tab,
                    inner.timeout_ms,
                    inner.poll_ms,
                )
                .await?;
                println!("{}", summary);
            }
            EdgeCommands::Back => println!("{}", edge::actions::click("back").await?),
            EdgeCommands::Forward => println!("{}", edge::actions::click("forward").await?),
            EdgeCommands::Reload => println!("{}", edge::actions::click("reload").await?),
            EdgeCommands::NewTab => {
                let page = edge::devtools::new_page("about:blank").await?;
                println!("opened edge new tab {:?} ({})", page.title, page.id);
            }
            EdgeCommands::Option(inner) => {
                let summary = edge::options::choose(
                    map_browser_option_prompt(inner.prompt),
                    map_browser_option_choice(inner.choice),
                )
                .await?;
                println!("{}", summary);
            }
            EdgeCommands::Current(inner) => {
                let summary = match inner.window_id.as_deref() {
                    Some(window_id) => {
                        edge::current::read_for_window(inner.json, Some(window_id)).await?
                    }
                    None => edge::current::read(inner.json).await?,
                };
                print!("{}", summary);
            }
            EdgeCommands::Tabs(inner) => match inner.command {
                ChromeTabsCommand::List => {
                    let summary = match inner.window_id.as_deref() {
                        Some(window_id) => {
                            edge::tabs::list_tabs_for_window(Some(window_id)).await?
                        }
                        None => edge::tabs::list_tabs().await?,
                    };
                    print!("{}", summary);
                }
                ChromeTabsCommand::Switch(tab) => {
                    if let Some(window_id) = inner.window_id.as_deref() {
                        let _ = edge::session::remember_target(window_id);
                    }
                    let target = match (tab.index, tab.title.as_deref()) {
                        (Some(index), None) => edge::tabs::TabSwitchTarget::Index(index),
                        (None, Some(title)) => {
                            edge::tabs::TabSwitchTarget::TitleContains(title.to_string())
                        }
                        (None, None) => anyhow::bail!(
                            "edge tabs switch needs either an index or --title <contains>"
                        ),
                        (Some(_), Some(_)) => unreachable!("clap enforces index/title exclusivity"),
                    };
                    let summary = edge::tabs::switch(target).await?;
                    println!("{}", summary);
                }
                ChromeTabsCommand::Close(tab) => {
                    if let Some(window_id) = inner.window_id.as_deref() {
                        let _ = edge::session::remember_target(window_id);
                    }
                    let summary = edge::tabs::close(tab.index).await?;
                    println!("{}", summary);
                }
            },
            EdgeCommands::Page(inner) => {
                if let Ok(window) = edge::window::find_edge_window(None) {
                    let _ = chrome::session::remember_browser_window_target(&window.id);
                }
                unsafe {
                    std::env::set_var("GUIBOT_BROWSER_WINDOW_MODE", "edge");
                }
                let edge_page_result: Result<()> = match inner.command {
                    ChromePageCommand::Inspect(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        let tree = edge::page::find::inspect(&scope).await?;
                        print!("{}", render::render_tree(&tree));
                        Ok(())
                    }
                    ChromePageCommand::Frames(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        let matches = edge::page::find::frames(&scope).await?;
                        print!("{}", render::render_live_matches(&matches));
                        Ok(())
                    }
                    ChromePageCommand::Find(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        let matches = edge::page::find::find(&scope, &inner.selectors).await?;
                        print!("{}", render::render_live_matches(&matches));
                        Ok(())
                    }
                    ChromePageCommand::Count(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::find::count(&scope, &inner.selectors).await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::Read(inner) => {
                        let scope = edge::page::root::PageScope::from_raw(
                            &inner.target.scope.frame_selectors,
                        )?;
                        let summary = if inner.value {
                            edge::page::actions::read_value(
                                &scope,
                                &inner.target.selectors,
                                inner.target.nth,
                            )
                            .await?
                        } else {
                            edge::page::actions::read_text(
                                &scope,
                                &inner.target.selectors,
                                inner.target.nth,
                            )
                            .await?
                        };
                        println!("{}", summary);
                        Ok(())
                    }
                    ChromePageCommand::Focus(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::focus(&scope, &inner.selectors, inner.nth).await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::Click(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::click(&scope, &inner.selectors, inner.nth).await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::Hover(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::hover(&scope, &inner.selectors, inner.nth).await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::DoubleClick(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::click_kind(
                                &scope,
                                &inner.selectors,
                                inner.nth,
                                edge::page::actions::PointerClickKind::Double,
                            )
                            .await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::RightClick(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::click_kind(
                                &scope,
                                &inner.selectors,
                                inner.nth,
                                edge::page::actions::PointerClickKind::Secondary,
                            )
                            .await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::ClickAndWait(inner) => {
                        let action_scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        let wait_scope = edge::page::root::PageScope::from_raw(
                            &inner.wait.wait_frame_selectors,
                        )?;
                        let summary = edge::page::flow::click_and_wait(
                            &action_scope,
                            &inner.selectors,
                            edge::page::flow::PostActionWait {
                                scope: &wait_scope,
                                selectors: &inner.wait.wait_selectors,
                                conditions: edge::page::wait::PageWaitConditions {
                                    text: inner.wait.text.as_deref(),
                                    title_contains: inner.wait.title_contains.as_deref(),
                                    url_contains: inner.wait.url_contains.as_deref(),
                                    disappear: inner.wait.disappear,
                                },
                                timing: edge::page::wait::PageWaitTiming {
                                    timeout_ms: inner.wait.timeout_ms,
                                    poll_ms: inner.wait.poll_ms,
                                },
                            },
                        )
                        .await?;
                        println!("{}", summary);
                        Ok(())
                    }
                    ChromePageCommand::Type(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::type_text(&scope, &inner.selectors, &inner.text)
                                .await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::Key(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::press_key(&scope, &inner.selectors, &inner.key)
                                .await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::PressEnter(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::press_enter(&scope, &inner.selectors).await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::SubmitAndWait(inner) => {
                        let action_scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        let wait_scope = edge::page::root::PageScope::from_raw(
                            &inner.wait.wait_frame_selectors,
                        )?;
                        let summary = edge::page::flow::submit_and_wait(
                            &action_scope,
                            &inner.selectors,
                            edge::page::flow::PostActionWait {
                                scope: &wait_scope,
                                selectors: &inner.wait.wait_selectors,
                                conditions: edge::page::wait::PageWaitConditions {
                                    text: inner.wait.text.as_deref(),
                                    title_contains: inner.wait.title_contains.as_deref(),
                                    url_contains: inner.wait.url_contains.as_deref(),
                                    disappear: inner.wait.disappear,
                                },
                                timing: edge::page::wait::PageWaitTiming {
                                    timeout_ms: inner.wait.timeout_ms,
                                    poll_ms: inner.wait.poll_ms,
                                },
                            },
                        )
                        .await?;
                        println!("{}", summary);
                        Ok(())
                    }
                    ChromePageCommand::Check(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::check(&scope, &inner.selectors).await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::Uncheck(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::uncheck(&scope, &inner.selectors).await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::SelectOption(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::select_option(
                                &scope,
                                &inner.selectors,
                                &inner.option,
                                inner.nth
                            )
                            .await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::Upload(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        println!(
                            "{}",
                            edge::page::actions::upload(&scope, &inner.selectors, &inner.path)
                                .await?
                        );
                        Ok(())
                    }
                    ChromePageCommand::Scroll(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        let summary = if inner.into_view {
                            edge::page::actions::scroll_target_into_view(
                                &scope,
                                &inner.selectors,
                                inner.nth,
                            )
                            .await?
                        } else {
                            edge::page::actions::scroll_window(
                                &scope,
                                map_page_scroll_direction(inner.direction),
                                inner.amount,
                            )
                            .await?
                        };
                        println!("{}", summary);
                        Ok(())
                    }
                    ChromePageCommand::Screenshot(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        let summary = match edge::page::screenshot::capture(
                            &scope,
                            &inner.output,
                            &inner.selectors,
                            inner.nth,
                        )
                        .await
                        {
                            Ok(summary) => summary,
                            Err(err) => {
                                let fallback_summary = edge::screenshot::capture_mode(
                                    &inner.output,
                                    None,
                                    edge::screenshot::ScreenshotMode::Window,
                                )
                                .await?;
                                format!(
                                    "edge page screenshot fallback to window capture: {} | reason: {}",
                                    fallback_summary, err
                                )
                            }
                        };
                        println!("{}", summary);
                        Ok(())
                    }
                    ChromePageCommand::Wait(inner) => {
                        let scope =
                            edge::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                        let summary = if let Some(state) = inner.state {
                            edge::page::wait::wait_for_state(
                                &scope,
                                &inner.selectors,
                                inner.nth,
                                map_page_state(state),
                                inner.timeout_ms,
                                inner.poll_ms,
                            )
                            .await?
                        } else {
                            edge::page::wait::wait_for_target(
                                &scope,
                                &inner.selectors,
                                edge::page::wait::PageWaitConditions {
                                    text: inner.text.as_deref(),
                                    title_contains: inner.title_contains.as_deref(),
                                    url_contains: inner.url_contains.as_deref(),
                                    disappear: inner.disappear,
                                },
                                edge::page::wait::PageWaitTiming {
                                    timeout_ms: inner.timeout_ms,
                                    poll_ms: inner.poll_ms,
                                },
                            )
                            .await?
                        };
                        println!("{}", summary);
                        Ok(())
                    }
                };
                unsafe {
                    std::env::remove_var("GUIBOT_BROWSER_WINDOW_MODE");
                }
                edge_page_result?
            }
        },
        Commands::Firefox(args) => match args.command {
            FirefoxCommands::Launch(inner) => {
                let summary = firefox::launch::launch_with_flavor(
                    firefox_flavor,
                    inner.url.as_deref(),
                    inner.profile.as_deref(),
                    inner.timeout_ms,
                    inner.poll_ms,
                )
                .await?;
                println!("{}", summary);
            }
            FirefoxCommands::Attach(inner) => {
                let summary =
                    firefox::attach::attach(inner.json, inner.window_id.as_deref()).await?;
                print!("{}", summary);
            }
            FirefoxCommands::Windows => {
                let windows = firefox::window::list_firefox_windows(None)?;
                for window in windows {
                    println!(
                        "{} {} ({}x{} at {},{})",
                        window.id, window.name, window.width, window.height, window.x, window.y
                    );
                }
            }
            FirefoxCommands::Locate(inner) => {
                let node = firefox::actions::click::resolve_locator(&inner.locator).await?;
                let locator = chrome::locators::ChromeLocator::parse(&inner.locator)?;
                print!(
                    "{}",
                    render::render_chrome_locator(locator.canonical_name(), &node)
                );
            }
            FirefoxCommands::Screenshot(inner) => {
                let mode = if inner.active {
                    firefox::screenshot::ScreenshotMode::Active
                } else {
                    firefox::screenshot::ScreenshotMode::Window
                };
                let summary =
                    firefox::screenshot::capture_mode(&inner.output, inner.query.as_deref(), mode)
                        .await?;
                println!("{}", summary);
            }
            FirefoxCommands::Resize(inner) => {
                let summary = firefox::resize::resize(
                    inner.query.as_deref(),
                    inner.preset.map(map_resize_preset),
                    inner.width,
                    inner.height,
                )?;
                println!("{}", summary);
            }
            FirefoxCommands::Focus(inner) => {
                let summary = firefox::actions::focus(&inner.locator).await?;
                println!("{}", summary);
            }
            FirefoxCommands::Click(inner) => {
                let summary = firefox::actions::click(&inner.locator).await?;
                println!("{}", summary);
            }
            FirefoxCommands::Type(inner) => {
                let summary = firefox::actions::type_text(&inner.locator, &inner.text).await?;
                println!("{}", summary);
            }
            FirefoxCommands::Key(inner) => {
                let summary = firefox::actions::press_key(&inner.locator, &inner.key).await?;
                println!("{}", summary);
            }
            FirefoxCommands::PressEnter(inner) => {
                let summary = firefox::actions::press_enter(inner.locator.as_deref()).await?;
                println!("{}", summary);
            }
            FirefoxCommands::Hold(inner) => {
                let summary =
                    firefox::actions::hold(&inner.key, inner.duration_ms, inner.locator.as_deref())
                        .await?;
                println!("{}", summary);
            }
            FirefoxCommands::Scroll(inner) => {
                let scope = firefox::page::root::PageScope::default();
                let summary = firefox::page::actions::scroll_window(
                    &scope,
                    map_page_scroll_direction_firefox(inner.direction),
                    inner.amount,
                )
                .await?;
                println!("{}", summary);
            }
            FirefoxCommands::ScrollTo(inner) => {
                let window = firefox::window::find_firefox_window(None)?;
                let activation_note = firefox::actions::context::activate_window_note(&window.id);
                crate::window::send_key(&window.id, map_scroll_to_key(inner.target))?;
                println!(
                    "scrolled page window {} to {:?} ({})",
                    window.id, inner.target, activation_note
                );
            }
            FirefoxCommands::Wait(inner) => {
                let summary = match inner.target {
                    ChromeWaitTarget::Locator(wait) => {
                        firefox::wait::wait_for_locator(
                            &wait.locator,
                            wait.timeout_ms,
                            wait.poll_ms,
                        )
                        .await?
                    }
                    ChromeWaitTarget::TitleChange(wait) => {
                        firefox::wait::wait_for_title_change(
                            wait.from.as_deref(),
                            wait.timeout_ms,
                            wait.poll_ms,
                        )
                        .await?
                    }
                    ChromeWaitTarget::UrlChange(wait) => {
                        firefox::wait::wait_for_url_change(
                            wait.from.as_deref(),
                            wait.timeout_ms,
                            wait.poll_ms,
                        )
                        .await?
                    }
                };
                println!("{}", summary);
            }
            FirefoxCommands::Goto(inner) => {
                let summary = firefox::goto::navigate(
                    firefox_flavor,
                    &inner.url,
                    inner.new_tab,
                    inner.timeout_ms,
                    inner.poll_ms,
                )
                .await?;
                println!("{}", summary);
            }
            FirefoxCommands::Back => println!("{}", firefox::actions::click("back").await?),
            FirefoxCommands::Forward => println!("{}", firefox::actions::click("forward").await?),
            FirefoxCommands::Reload => println!("{}", firefox::actions::click("reload").await?),
            FirefoxCommands::NewTab => {
                let summary = firefox::goto::open_new_tab(firefox_flavor, 5_000, 200).await?;
                println!("{}", summary);
            }
            FirefoxCommands::Option(inner) => {
                let summary = firefox::options::choose(
                    map_browser_option_prompt(inner.prompt),
                    map_browser_option_choice(inner.choice),
                )
                .await?;
                println!("{}", summary);
            }
            FirefoxCommands::Current(inner) => {
                let summary = match inner.window_id.as_deref() {
                    Some(window_id) => {
                        firefox::current::read_for_window(inner.json, Some(window_id)).await?
                    }
                    None => firefox::current::read(inner.json).await?,
                };
                print!("{}", summary);
            }
            FirefoxCommands::Tabs(inner) => match inner.command {
                ChromeTabsCommand::List => {
                    let summary = match inner.window_id.as_deref() {
                        Some(window_id) => {
                            firefox::tabs::list_tabs_for_window(Some(window_id)).await?
                        }
                        None => firefox::tabs::list_tabs().await?,
                    };
                    print!("{}", summary);
                }
                ChromeTabsCommand::Switch(tab) => {
                    if let Some(window_id) = inner.window_id.as_deref() {
                        let _ = firefox::session::remember_target(window_id);
                    }
                    let target = match (tab.index, tab.title.as_deref()) {
                        (Some(index), None) => firefox::tabs::TabSwitchTarget::Index(index),
                        (None, Some(title)) => {
                            firefox::tabs::TabSwitchTarget::TitleContains(title.to_string())
                        }
                        (None, None) => anyhow::bail!(
                            "{} tabs switch needs either an index or --title <contains>",
                            firefox_flavor.label()
                        ),
                        (Some(_), Some(_)) => unreachable!("clap enforces index/title exclusivity"),
                    };
                    let summary = firefox::tabs::switch(target).await?;
                    println!("{}", summary);
                }
                ChromeTabsCommand::Close(tab) => {
                    if let Some(window_id) = inner.window_id.as_deref() {
                        let _ = firefox::session::remember_target(window_id);
                    }
                    let summary = firefox::tabs::close(tab.index).await?;
                    println!("{}", summary);
                }
            },
            FirefoxCommands::Page(inner) => match inner.command {
                ChromePageCommand::Inspect(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let tree = firefox::page::find::inspect(&scope).await?;
                    print!("{}", render::render_tree(&tree));
                }
                ChromePageCommand::Frames(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let matches = firefox::page::find::frames(&scope).await?;
                    print!("{}", render::render_live_matches(&matches));
                }
                ChromePageCommand::Find(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let matches = firefox::page::find::find(&scope, &inner.selectors).await?;
                    print!("{}", render::render_live_matches(&matches));
                }
                ChromePageCommand::Count(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::find::count(&scope, &inner.selectors).await?
                    );
                }
                ChromePageCommand::Read(inner) => {
                    let scope = firefox::page::root::PageScope::from_raw(
                        &inner.target.scope.frame_selectors,
                    )?;
                    let summary = if inner.value {
                        firefox::page::actions::read_value(
                            &scope,
                            &inner.target.selectors,
                            inner.target.nth,
                        )
                        .await?
                    } else {
                        firefox::page::actions::read_text(
                            &scope,
                            &inner.target.selectors,
                            inner.target.nth,
                        )
                        .await?
                    };
                    println!("{}", summary);
                }
                ChromePageCommand::Focus(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::focus(&scope, &inner.selectors, inner.nth).await?
                    );
                }
                ChromePageCommand::Click(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::click(&scope, &inner.selectors, inner.nth).await?
                    );
                }
                ChromePageCommand::Hover(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::hover(&scope, &inner.selectors, inner.nth).await?
                    );
                }
                ChromePageCommand::DoubleClick(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::click_kind(
                            &scope,
                            &inner.selectors,
                            inner.nth,
                            firefox::page::actions::PointerClickKind::Double,
                        )
                        .await?
                    );
                }
                ChromePageCommand::RightClick(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::click_kind(
                            &scope,
                            &inner.selectors,
                            inner.nth,
                            firefox::page::actions::PointerClickKind::Secondary,
                        )
                        .await?
                    );
                }
                ChromePageCommand::ClickAndWait(inner) => {
                    let action_scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let wait_scope =
                        firefox::page::root::PageScope::from_raw(&inner.wait.wait_frame_selectors)?;
                    let summary = firefox::page::flow::click_and_wait(
                        &action_scope,
                        &inner.selectors,
                        firefox::page::flow::PostActionWait {
                            scope: &wait_scope,
                            selectors: &inner.wait.wait_selectors,
                            conditions: firefox::page::wait::PageWaitConditions {
                                text: inner.wait.text.as_deref(),
                                title_contains: inner.wait.title_contains.as_deref(),
                                url_contains: inner.wait.url_contains.as_deref(),
                                disappear: inner.wait.disappear,
                            },
                            timing: firefox::page::wait::PageWaitTiming {
                                timeout_ms: inner.wait.timeout_ms,
                                poll_ms: inner.wait.poll_ms,
                            },
                        },
                    )
                    .await?;
                    println!("{}", summary);
                }
                ChromePageCommand::Type(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::type_text(&scope, &inner.selectors, &inner.text)
                            .await?
                    );
                }
                ChromePageCommand::Key(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::press_key(&scope, &inner.selectors, &inner.key)
                            .await?
                    );
                }
                ChromePageCommand::PressEnter(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::press_enter(&scope, &inner.selectors).await?
                    );
                }
                ChromePageCommand::SubmitAndWait(inner) => {
                    let action_scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let wait_scope =
                        firefox::page::root::PageScope::from_raw(&inner.wait.wait_frame_selectors)?;
                    let summary = firefox::page::flow::submit_and_wait(
                        &action_scope,
                        &inner.selectors,
                        firefox::page::flow::PostActionWait {
                            scope: &wait_scope,
                            selectors: &inner.wait.wait_selectors,
                            conditions: firefox::page::wait::PageWaitConditions {
                                text: inner.wait.text.as_deref(),
                                title_contains: inner.wait.title_contains.as_deref(),
                                url_contains: inner.wait.url_contains.as_deref(),
                                disappear: inner.wait.disappear,
                            },
                            timing: firefox::page::wait::PageWaitTiming {
                                timeout_ms: inner.wait.timeout_ms,
                                poll_ms: inner.wait.poll_ms,
                            },
                        },
                    )
                    .await?;
                    println!("{}", summary);
                }
                ChromePageCommand::Check(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::check(&scope, &inner.selectors).await?
                    );
                }
                ChromePageCommand::Uncheck(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::uncheck(&scope, &inner.selectors).await?
                    );
                }
                ChromePageCommand::SelectOption(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::select_option(
                            &scope,
                            &inner.selectors,
                            &inner.option,
                            inner.nth
                        )
                        .await?
                    );
                }
                ChromePageCommand::Upload(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        firefox::page::actions::upload(&scope, &inner.selectors, &inner.path)
                            .await?
                    );
                }
                ChromePageCommand::Scroll(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary = if inner.into_view {
                        firefox::page::actions::scroll_target_into_view(
                            &scope,
                            &inner.selectors,
                            inner.nth,
                        )
                        .await?
                    } else {
                        firefox::page::actions::scroll_window(
                            &scope,
                            map_page_scroll_direction_firefox(inner.direction),
                            inner.amount,
                        )
                        .await?
                    };
                    println!("{}", summary);
                }
                ChromePageCommand::Screenshot(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary = match firefox::page::screenshot::capture(
                        &scope,
                        &inner.output,
                        &inner.selectors,
                        inner.nth,
                    )
                    .await
                    {
                        Ok(summary) => summary,
                        Err(err) => {
                            let fallback_summary = firefox::screenshot::capture_mode(
                                &inner.output,
                                None,
                                firefox::screenshot::ScreenshotMode::Window,
                            )
                            .await?;
                            format!(
                                "firefox page screenshot fallback to window capture: {} | reason: {}",
                                fallback_summary, err
                            )
                        }
                    };
                    println!("{}", summary);
                }
                ChromePageCommand::Wait(inner) => {
                    let scope =
                        firefox::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary = if let Some(state) = inner.state {
                        firefox::page::wait::wait_for_state(
                            &scope,
                            &inner.selectors,
                            inner.nth,
                            map_page_state_firefox(state),
                            inner.timeout_ms,
                            inner.poll_ms,
                        )
                        .await?
                    } else {
                        firefox::page::wait::wait_for_target(
                            &scope,
                            &inner.selectors,
                            firefox::page::wait::PageWaitConditions {
                                text: inner.text.as_deref(),
                                title_contains: inner.title_contains.as_deref(),
                                url_contains: inner.url_contains.as_deref(),
                                disappear: inner.disappear,
                            },
                            firefox::page::wait::PageWaitTiming {
                                timeout_ms: inner.timeout_ms,
                                poll_ms: inner.poll_ms,
                            },
                        )
                        .await?
                    };
                    println!("{}", summary);
                }
            },
        },
        Commands::Chrome(args) => match args.command {
            ChromeCommands::Launch(inner) => {
                let summary = chrome::launch::launch(
                    inner.url.as_deref(),
                    inner.profile.as_deref(),
                    inner.timeout_ms,
                    inner.poll_ms,
                )
                .await?;
                println!("{}", summary);
            }
            ChromeCommands::Attach(inner) => {
                let summary =
                    chrome::attach::attach(inner.json, inner.window_id.as_deref()).await?;
                print!("{}", summary);
            }
            ChromeCommands::Windows => {
                let summary = chrome::windows::list()?;
                print!("{}", summary);
            }
            ChromeCommands::Locate(inner) => {
                let located = chrome::locators::locate(&inner.locator).await?;
                print!(
                    "{}",
                    render::render_chrome_locator(located.locator.canonical_name(), &located.node)
                );
            }
            ChromeCommands::Screenshot(inner) => {
                let mode = if inner.active {
                    chrome::screenshot::ScreenshotMode::Active
                } else {
                    chrome::screenshot::ScreenshotMode::Window
                };
                let summary =
                    chrome::screenshot::capture_mode(&inner.output, inner.query.as_deref(), mode)
                        .await?;
                println!("{}", summary);
            }
            ChromeCommands::Resize(inner) => {
                let summary = chrome::resize::resize(
                    inner.query.as_deref(),
                    inner.preset.map(map_resize_preset),
                    inner.width,
                    inner.height,
                )?;
                println!("{}", summary);
            }
            ChromeCommands::Focus(inner) => {
                let summary = chrome::actions::focus(&inner.locator).await?;
                println!("{}", summary);
            }
            ChromeCommands::Click(inner) => {
                let summary = chrome::actions::click(&inner.locator).await?;
                println!("{}", summary);
            }
            ChromeCommands::Type(inner) => {
                let summary = chrome::actions::type_text(&inner.locator, &inner.text).await?;
                println!("{}", summary);
            }
            ChromeCommands::Key(inner) => {
                let summary = chrome::actions::press_key(&inner.locator, &inner.key).await?;
                println!("{}", summary);
            }
            ChromeCommands::PressEnter(inner) => {
                let summary = chrome::actions::press_enter(inner.locator.as_deref()).await?;
                println!("{}", summary);
            }
            ChromeCommands::Hold(inner) => {
                let summary =
                    chrome::actions::hold(&inner.key, inner.duration_ms, inner.locator.as_deref())
                        .await?;
                println!("{}", summary);
            }
            ChromeCommands::Scroll(inner) => {
                let scope = chrome::page::root::PageScope::default();
                let summary = chrome::page::actions::scroll_window(
                    &scope,
                    map_page_scroll_direction(inner.direction),
                    inner.amount,
                )
                .await?;
                println!("{}", summary);
            }
            ChromeCommands::ScrollTo(inner) => {
                let window = chrome::window::find_browser_window(None)?;
                let activation_note = chrome::actions::context::activate_window_note(&window.id);
                crate::window::send_key(&window.id, map_scroll_to_key(inner.target))?;
                println!(
                    "scrolled page window {} to {:?} ({})",
                    window.id, inner.target, activation_note
                );
            }
            ChromeCommands::Goto(inner) => {
                let summary = chrome::goto::navigate(
                    &inner.url,
                    inner.new_tab,
                    inner.timeout_ms,
                    inner.poll_ms,
                )
                .await?;
                println!("{}", summary);
            }
            ChromeCommands::Back => println!("{}", chrome::actions::click("back").await?),
            ChromeCommands::Forward => println!("{}", chrome::actions::click("forward").await?),
            ChromeCommands::Reload => println!("{}", chrome::actions::click("reload").await?),
            ChromeCommands::NewTab => {
                let summary = chrome::goto::navigate(
                    "about:blank",
                    true,
                    chrome::wait::default_timeout_ms(),
                    chrome::wait::default_poll_ms(),
                )
                .await?;
                println!("{}", summary);
            }
            ChromeCommands::Option(inner) => {
                let summary = chrome::options::choose(
                    map_browser_option_prompt(inner.prompt),
                    map_browser_option_choice(inner.choice),
                )
                .await?;
                println!("{}", summary);
            }
            ChromeCommands::Current(inner) => {
                let summary = chrome::current::read(inner.json).await?;
                print!("{}", summary);
            }
            ChromeCommands::Tabs(inner) => match inner.command {
                ChromeTabsCommand::List => {
                    let summary = chrome::tabs::list_tabs().await?;
                    print!("{}", summary);
                }
                ChromeTabsCommand::Switch(tab) => {
                    let target = match (tab.index, tab.title.as_deref()) {
                        (Some(index), None) => chrome::tabs::TabSwitchTarget::Index(index),
                        (None, Some(title)) => {
                            chrome::tabs::TabSwitchTarget::TitleContains(title.to_string())
                        }
                        (None, None) => anyhow::bail!(
                            "chrome tabs switch needs either an index or --title <contains>"
                        ),
                        (Some(_), Some(_)) => unreachable!("clap enforces index/title exclusivity"),
                    };
                    let summary = chrome::tabs::switch(target).await?;
                    println!("{}", summary);
                }
                ChromeTabsCommand::Close(tab) => {
                    let summary = chrome::tabs::close(tab.index).await?;
                    println!("{}", summary);
                }
            },
            ChromeCommands::Page(inner) => match inner.command {
                ChromePageCommand::Inspect(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let tree = chrome::page::find::inspect(&scope).await?;
                    print!("{}", render::render_tree(&tree));
                }
                ChromePageCommand::Frames(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let matches = chrome::page::find::frames(&scope).await?;
                    print!("{}", render::render_live_matches(&matches));
                }
                ChromePageCommand::Find(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let matches = chrome::page::find::find(&scope, &inner.selectors).await?;
                    print!("{}", render::render_live_matches(&matches));
                }
                ChromePageCommand::Count(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        chrome::page::find::count(&scope, &inner.selectors).await?
                    );
                }
                ChromePageCommand::Read(inner) => {
                    let scope = chrome::page::root::PageScope::from_raw(
                        &inner.target.scope.frame_selectors,
                    )?;
                    let summary = if inner.value {
                        chrome::page::actions::read_value(
                            &scope,
                            &inner.target.selectors,
                            inner.target.nth,
                        )
                        .await?
                    } else {
                        chrome::page::actions::read_text(
                            &scope,
                            &inner.target.selectors,
                            inner.target.nth,
                        )
                        .await?
                    };
                    println!("{}", summary);
                }
                ChromePageCommand::Focus(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        chrome::page::actions::focus(&scope, &inner.selectors, inner.nth).await?
                    );
                }
                ChromePageCommand::Click(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        chrome::page::actions::click(&scope, &inner.selectors, inner.nth).await?
                    );
                }
                ChromePageCommand::Hover(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        chrome::page::actions::hover(&scope, &inner.selectors, inner.nth).await?
                    );
                }
                ChromePageCommand::DoubleClick(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        chrome::page::actions::click_kind(
                            &scope,
                            &inner.selectors,
                            inner.nth,
                            chrome::page::actions::PointerClickKind::Double,
                        )
                        .await?
                    );
                }
                ChromePageCommand::RightClick(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    println!(
                        "{}",
                        chrome::page::actions::click_kind(
                            &scope,
                            &inner.selectors,
                            inner.nth,
                            chrome::page::actions::PointerClickKind::Secondary,
                        )
                        .await?
                    );
                }
                ChromePageCommand::ClickAndWait(inner) => {
                    let action_scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let wait_scope =
                        chrome::page::root::PageScope::from_raw(&inner.wait.wait_frame_selectors)?;
                    let summary = chrome::page::flow::click_and_wait(
                        &action_scope,
                        &inner.selectors,
                        chrome::page::flow::PostActionWait {
                            scope: &wait_scope,
                            selectors: &inner.wait.wait_selectors,
                            conditions: chrome::page::wait::PageWaitConditions {
                                text: inner.wait.text.as_deref(),
                                title_contains: inner.wait.title_contains.as_deref(),
                                url_contains: inner.wait.url_contains.as_deref(),
                                disappear: inner.wait.disappear,
                            },
                            timing: chrome::page::wait::PageWaitTiming {
                                timeout_ms: inner.wait.timeout_ms,
                                poll_ms: inner.wait.poll_ms,
                            },
                        },
                    )
                    .await?;
                    println!("{}", summary);
                }
                ChromePageCommand::Type(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary =
                        chrome::page::actions::type_text(&scope, &inner.selectors, &inner.text)
                            .await?;
                    println!("{}", summary);
                }
                ChromePageCommand::Key(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary =
                        chrome::page::actions::press_key(&scope, &inner.selectors, &inner.key)
                            .await?;
                    println!("{}", summary);
                }
                ChromePageCommand::PressEnter(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary =
                        chrome::page::actions::press_enter(&scope, &inner.selectors).await?;
                    println!("{}", summary);
                }
                ChromePageCommand::SubmitAndWait(inner) => {
                    let action_scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let wait_scope =
                        chrome::page::root::PageScope::from_raw(&inner.wait.wait_frame_selectors)?;
                    let summary = chrome::page::flow::submit_and_wait(
                        &action_scope,
                        &inner.selectors,
                        chrome::page::flow::PostActionWait {
                            scope: &wait_scope,
                            selectors: &inner.wait.wait_selectors,
                            conditions: chrome::page::wait::PageWaitConditions {
                                text: inner.wait.text.as_deref(),
                                title_contains: inner.wait.title_contains.as_deref(),
                                url_contains: inner.wait.url_contains.as_deref(),
                                disappear: inner.wait.disappear,
                            },
                            timing: chrome::page::wait::PageWaitTiming {
                                timeout_ms: inner.wait.timeout_ms,
                                poll_ms: inner.wait.poll_ms,
                            },
                        },
                    )
                    .await?;
                    println!("{}", summary);
                }
                ChromePageCommand::Check(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary = chrome::page::actions::check(&scope, &inner.selectors).await?;
                    println!("{}", summary);
                }
                ChromePageCommand::Uncheck(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary = chrome::page::actions::uncheck(&scope, &inner.selectors).await?;
                    println!("{}", summary);
                }
                ChromePageCommand::SelectOption(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary = chrome::page::actions::select_option(
                        &scope,
                        &inner.selectors,
                        &inner.option,
                        inner.nth,
                    )
                    .await?;
                    println!("{}", summary);
                }
                ChromePageCommand::Upload(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary =
                        chrome::page::actions::upload(&scope, &inner.selectors, &inner.path)
                            .await?;
                    println!("{}", summary);
                }
                ChromePageCommand::Scroll(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary = if inner.into_view {
                        chrome::page::actions::scroll_target_into_view(
                            &scope,
                            &inner.selectors,
                            inner.nth,
                        )
                        .await?
                    } else {
                        chrome::page::actions::scroll_window(
                            &scope,
                            map_page_scroll_direction(inner.direction),
                            inner.amount,
                        )
                        .await?
                    };
                    println!("{}", summary);
                }
                ChromePageCommand::Screenshot(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary = chrome::page::screenshot::capture(
                        &scope,
                        &inner.output,
                        &inner.selectors,
                        inner.nth,
                    )
                    .await?;
                    println!("{}", summary);
                }
                ChromePageCommand::Wait(inner) => {
                    let scope =
                        chrome::page::root::PageScope::from_raw(&inner.scope.frame_selectors)?;
                    let summary = if let Some(state) = inner.state {
                        chrome::page::wait::wait_for_state(
                            &scope,
                            &inner.selectors,
                            inner.nth,
                            map_page_state(state),
                            inner.timeout_ms,
                            inner.poll_ms,
                        )
                        .await?
                    } else {
                        chrome::page::wait::wait_for_target(
                            &scope,
                            &inner.selectors,
                            chrome::page::wait::PageWaitConditions {
                                text: inner.text.as_deref(),
                                title_contains: inner.title_contains.as_deref(),
                                url_contains: inner.url_contains.as_deref(),
                                disappear: inner.disappear,
                            },
                            chrome::page::wait::PageWaitTiming {
                                timeout_ms: inner.timeout_ms,
                                poll_ms: inner.poll_ms,
                            },
                        )
                        .await?
                    };
                    println!("{}", summary);
                }
            },
            ChromeCommands::Wait(inner) => {
                let summary = match inner.target {
                    ChromeWaitTarget::Locator(wait) => {
                        chrome::wait::wait_for_locator(&wait.locator, wait.timeout_ms, wait.poll_ms)
                            .await?
                    }
                    ChromeWaitTarget::TitleChange(wait) => {
                        chrome::wait::wait_for_title_change(
                            wait.from.as_deref(),
                            wait.timeout_ms,
                            wait.poll_ms,
                        )
                        .await?
                    }
                    ChromeWaitTarget::UrlChange(wait) => {
                        chrome::wait::wait_for_url_change(
                            wait.from.as_deref(),
                            wait.timeout_ms,
                            wait.poll_ms,
                        )
                        .await?
                    }
                };
                println!("{}", summary);
            }
        },
    }

    Ok(())
}

fn current_firefox_flavor() -> firefox::launch::BrowserFlavor {
    match std::env::args().nth(1).as_deref() {
        Some("camoufox") => firefox::launch::BrowserFlavor::Camoufox,
        _ => firefox::launch::BrowserFlavor::Firefox,
    }
}

fn map_page_state_firefox(state: ChromePageStateArg) -> firefox::page::wait::PageStateWait {
    match state {
        ChromePageStateArg::Focused => firefox::page::wait::PageStateWait::Focused,
        ChromePageStateArg::Checked => firefox::page::wait::PageStateWait::Checked,
        ChromePageStateArg::Enabled => firefox::page::wait::PageStateWait::Enabled,
        ChromePageStateArg::Disabled => firefox::page::wait::PageStateWait::Disabled,
        ChromePageStateArg::Expanded => firefox::page::wait::PageStateWait::Expanded,
        ChromePageStateArg::Collapsed => firefox::page::wait::PageStateWait::Collapsed,
    }
}

fn map_page_scroll_direction_firefox(
    direction: ChromePageScrollDirectionArg,
) -> firefox::page::actions::PageScrollDirection {
    match direction {
        ChromePageScrollDirectionArg::Up => firefox::page::actions::PageScrollDirection::Up,
        ChromePageScrollDirectionArg::Down => firefox::page::actions::PageScrollDirection::Down,
        ChromePageScrollDirectionArg::Left => firefox::page::actions::PageScrollDirection::Left,
        ChromePageScrollDirectionArg::Right => firefox::page::actions::PageScrollDirection::Right,
    }
}

fn map_page_state(state: ChromePageStateArg) -> chrome::page::wait::PageStateWait {
    match state {
        ChromePageStateArg::Focused => chrome::page::wait::PageStateWait::Focused,
        ChromePageStateArg::Checked => chrome::page::wait::PageStateWait::Checked,
        ChromePageStateArg::Enabled => chrome::page::wait::PageStateWait::Enabled,
        ChromePageStateArg::Disabled => chrome::page::wait::PageStateWait::Disabled,
        ChromePageStateArg::Expanded => chrome::page::wait::PageStateWait::Expanded,
        ChromePageStateArg::Collapsed => chrome::page::wait::PageStateWait::Collapsed,
    }
}

fn map_page_scroll_direction(
    direction: ChromePageScrollDirectionArg,
) -> chrome::page::actions::PageScrollDirection {
    match direction {
        ChromePageScrollDirectionArg::Up => chrome::page::actions::PageScrollDirection::Up,
        ChromePageScrollDirectionArg::Down => chrome::page::actions::PageScrollDirection::Down,
        ChromePageScrollDirectionArg::Left => chrome::page::actions::PageScrollDirection::Left,
        ChromePageScrollDirectionArg::Right => chrome::page::actions::PageScrollDirection::Right,
    }
}

fn map_scroll_to_key(target: BrowserScrollTargetArg) -> &'static str {
    match target {
        BrowserScrollTargetArg::Top => "Home",
        BrowserScrollTargetArg::Bottom => "End",
    }
}

fn map_resize_preset(preset: ChromeResizePresetArg) -> chrome::resize::ResizePreset {
    match preset {
        ChromeResizePresetArg::Desktop => chrome::resize::ResizePreset::Desktop,
        ChromeResizePresetArg::Tablet => chrome::resize::ResizePreset::Tablet,
        ChromeResizePresetArg::Mobile => chrome::resize::ResizePreset::Mobile,
    }
}

fn map_browser_option_prompt(
    prompt: BrowserOptionPromptArg,
) -> browser_options::BrowserOptionPrompt {
    match prompt {
        BrowserOptionPromptArg::LeaveSite => browser_options::BrowserOptionPrompt::LeaveSite,
        BrowserOptionPromptArg::SavePassword => browser_options::BrowserOptionPrompt::SavePassword,
    }
}

fn map_browser_option_choice(
    choice: BrowserOptionChoiceArg,
) -> browser_options::BrowserOptionChoice {
    match choice {
        BrowserOptionChoiceArg::Cancel => browser_options::BrowserOptionChoice::Cancel,
        BrowserOptionChoiceArg::Leave => browser_options::BrowserOptionChoice::Leave,
        BrowserOptionChoiceArg::Never => browser_options::BrowserOptionChoice::Never,
        BrowserOptionChoiceArg::Save => browser_options::BrowserOptionChoice::Save,
    }
}

#[cfg(test)]
mod tests {
    use super::{BrowserScrollTargetArg, map_scroll_to_key};

    #[test]
    fn maps_scroll_to_targets_to_home_and_end() {
        assert_eq!(map_scroll_to_key(BrowserScrollTargetArg::Top), "Home");
        assert_eq!(map_scroll_to_key(BrowserScrollTargetArg::Bottom), "End");
    }
}
