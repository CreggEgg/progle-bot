use anyhow::anyhow;
use chrono::{self, Datelike};
use pom::parser::*;
use pom::set::Set;
use serenity::builder::CreateApplicationCommandOption;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::prelude::application_command::CommandDataOptionValue;
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::{async_trait, model::guild};
use shuttle_runtime::CustomError;
use shuttle_secrets::SecretStore;
use sqlx::*;
use tracing::{error, info};

struct ProgleResult {
    code_game: bool,
    attempts: u16,
}

fn progle_result<'a>() -> Parser<'a, char, ProgleResult> {
    (tag("Found #progle language in ")
        * one_of("1234567890").repeat(0..).map(|number| {
            number
                .into_iter()
                .collect::<String>()
                .parse::<u16>()
                .unwrap()
        })
        - (tag(" attempts! 💥 Try and beat me 💥\n") | tag(" attempt! 💥 Try and beat me 💥\n"))
        + tag("Guess today's code snippet!").opt())
    .map(|(t1, t2)| ProgleResult {
        code_game: t2.is_some(),
        attempts: t1,
    }) - any().collect()
}

struct Bot {
    pool: PgPool,
}

async fn add_to_database(
    pool: &Pool<Postgres>,
    guild: Option<serenity::model::prelude::GuildId>,
    user: serenity::model::prelude::UserId,
    code_game: bool,
    attempts: u16,
) -> Option<()> {
    let guild = guild?.0 as i64;
    let user = user.0 as i64;
    info!("adding to database {guild} {user} {attempts} {code_game}");

    let is_linked_to_server = pool
        .fetch_optional(
            format!(
                "select server, person from membership where server = {} AND person = {};",
                guild, user
            )
            .as_str(),
        )
        .await
        .ok()?
        .is_some();

    if !is_linked_to_server {
        println!("adding membership {guild} {user} {attempts} {code_game}");
        pool.execute(format!("insert into membership values ({}, {})", guild, user).as_str())
            .await
            .ok()?;
    }

    let day = chrono::offset::Utc::now().num_days_from_ce();

    let was_attempt =
        pool.fetch_optional(format!("select (person,day,codemode) from attempts where person = {} and day = {} and codemode = {};",user,day,code_game).as_str())
            .await.ok()?.is_some();

    if !was_attempt {
        println!("adding attempt {guild} {user} {attempts} {code_game}");
        pool.execute(
            format!(
                "insert into attempts values ({},{},{},{});",
                user, day, attempts, code_game
            )
            .as_str(),
        )
        .await
        .ok()?;
    }

    println!("done adding {guild} {user} {attempts} {code_game}");

    Some(())
}

async fn get_users_averages_as_str(
    pool: &Pool<Postgres>,
    user: User,
    introstr: &str,
    introstrneg: &str,
) -> String {
    let user = user.id;

    let rets = pool.fetch_one(format!("select AVG(cast(numberofguess as Float)),person,codemode from attempts where codemode = {} and person = {} group by person, codemode ",true,user).as_str()).await;

    let average_codemode: Option<f64> = match rets {
        Ok(data) => Some(data.get(0)),
        Err(sqlx::Error::RowNotFound) => None,
        Err(e) => {
            eprint!("error: {e}");
            return "".to_string();
        }
    };

    let rets = pool.fetch_one(format!("select AVG(cast(numberofguess as Float)),person,codemode from attempts where codemode = {} and person = {} group by person, codemode ",false,user).as_str()).await;

    let average_classic: Option<f64> = match rets {
        Ok(data) => Some(data.get(0)),
        Err(sqlx::Error::RowNotFound) => None,
        Err(e) => {
            eprint!("error: {e}");
            return "".to_string();
        }
    };

    match (average_classic, average_codemode) {
        (Some(classic), Some(codemode)) => format!(
            "{introstr} an average of {} of classic and {} for code",
            classic, codemode
        ),
        (Some(classic), None) => format!("{introstr} an average of {} of classic", classic),
        (None, Some(codemode)) => format!("{introstr} an average of {} of codemode", codemode),
        (None, None) => format!("{introstrneg} sent any progle scores yet"),
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.content == "!hello" {
            if let Err(e) = msg.channel_id.say(&ctx.http, "world!").await {
                error!("Error sending message: {:?}", e);
            }
        }

        let content: Vec<char> = msg.content.chars().collect();
        let thing: &[char] = &content;
        let progle_result = progle_result().parse(thing);
        let guild = msg.guild_id;
        let user = msg.author.id;

        match progle_result {
            Ok(ProgleResult {
                code_game,
                attempts,
            }) => add_to_database(&self.pool, guild, user, code_game, attempts).await,
            Err(_) => Some(()),
        };
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);

        let guilds = ready.guilds;

        for guild in guilds.iter() {
            let guild_id = guild.id;
            info!("in guild: {}", guild_id);
            let res = GuildId::set_application_commands(&guild_id, &ctx.http, |commands| {
                commands.create_application_command(|command| {
                    command
                        .name("averages")
                        .description("get your averages for progle")
                        .create_option(|option| {
                            option
                                .kind(command::CommandOptionType::User)
                                .name("user")
                                .description("the user to get the averages of")
                        })
                })
            })
            .await;

            match res {
                Err(err) => eprintln!("error occured while getting ready: {err}"),
                Ok(_) => (),
            }
        }
    }
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        // check if the interaction is a command
        if let Interaction::ApplicationCommand(command) = interaction {
            let response_content = match command.data.name.as_str() {
                "averages" => match &command.data.options.get(0).map(|dat| dat.resolved.clone()) {
                    Some(Some(CommandDataOptionValue::User(user, _))) => {
                        get_users_averages_as_str(
                            &self.pool,
                            user.clone(),
                            format!("{} has ", user.name).to_str(),
                            format!("{} hasn't", user.name).to_str(),
                        )
                        .await
                    }
                    _ => {
                        get_users_averages_as_str(
                            &self.pool,
                            command.user.clone(),
                            "you have",
                            "you haven't",
                        )
                        .await
                    }
                },
                command => unreachable!("Unknown command: {}", command),
            };
            // send `response_content` to the discord server
            command
                .create_interaction_response(&ctx.http, |response| {
                    response
                        .kind(InteractionResponseType::ChannelMessageWithSource)
                        .interaction_response_data(|message| message.content(response_content))
                })
                .await
                .ok();
        }
    }
}

#[shuttle_runtime::main]
async fn serenity(
    #[shuttle_secrets::Secrets] secret_store: SecretStore,
    #[shuttle_shared_db::Postgres] pool: PgPool,
) -> shuttle_serenity::ShuttleSerenity {
    println!("{:?}", chrono::offset::Utc::now());
    pool.execute(include_str!("../schema.sql"))
        .await
        .map_err(CustomError::new)?;

    // Get the discord token set in `Secrets.toml`
    let token = if let Some(token) = secret_store.get("DISCORD_TOKEN") {
        token
    } else {
        return Err(anyhow!("'DISCORD_TOKEN' was not found").into());
    };

    // Set gateway intents, which decides what events the bot will be notified about
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;

    let client = Client::builder(&token, intents)
        .event_handler(Bot { pool: pool })
        .await
        .expect("Err creating client");

    Ok(client.into())
}
