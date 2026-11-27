# Accounts

An account is a login that an agent uses to reach its model: a subscription, an API key, or a company's single sign-on. You can sign in several accounts of one provider at the same time, a personal one and a work one for example, and each session runs on the one you choose. Credentials stay in this computer's credential store, never in {{ config_file }}.

Open **Settings, Accounts**. Each provider (Claude, Codex, OpenCode and the others your agents come from) has its own list with an **Add account** button. An account that needs signing in again says so and has a **Sign in** button; one that reports its limits shows how much of each is left and when it resets.

## Add an account

1. Press **Add account** beside the provider.
2. Choose how to sign in. What is offered depends on the provider: **Subscription** (sign in with the provider's plan), **API key** (paste a key from the provider's console), **Token first**, or **Single sign-on**. Each option says which agents it supports, and warns when it leaves a login on disk that the agent's own commands can read.
3. Give it a name. That is what the session picker calls it.
4. Press **Sign in**. A subscription or single sign-on opens the provider's page in your browser; an API key is typed into a form that only the host reads.

![The Add account dialog](../assets/screens/accounts-add-dark.png#only-dark)
![The Add account dialog](../assets/screens/accounts-add-light.png#only-light)

![The Add account dialog at phone width](../assets/screens/accounts-add-phone-dark.png#only-dark)
![The Add account dialog at phone width](../assets/screens/accounts-add-phone-light.png#only-light)

!!! example "Worked example: a work account and a personal one"
    1. Add a Claude account named after your own subscription and sign in.
    2. Add a second one with **API key**, named for the project that pays for it, and paste the key.
    3. Start a new session. The picker beside the agent's name lists the accounts that agent offers: choose one before the first message. Once the session has started the picker only says which account it runs on, and the other account stays signed in for other sessions.
    4. When one of them runs out of its weekly limit, its row in **Settings, Accounts** shows it, and you can start the next session on the other.

An account's row has a menu with the other actions, including removing it.

Accounts are also where a local model (Foundry Local) or a LiteLLM proxy is added; the [host's page](../host.md#accounts) lists the account kinds and their settings.

## Roles

A role is a pool named by the work: a planner, an executor, a reviewer, or any you define. It lists, in order, the agents, accounts and models that can do that work, and the first one with usage left serves it.

**Settings, Accounts, Roles** has a card for each role. Each member shows a chip: *Serving now*, *Available*, or why it cannot serve (its usage is used up until a time, it is not installed, it is not signed in, its model is not listed). **Add a role** asks for a name, a title, how a member is chosen (first with usage, most usage left, or in turn), what its models must report, whether a running conversation stays on its member, and the members in the order they are taken. A member is an agent, an account or pool, a model, an effort and a profile; the arrows reorder them. The host writes the role into the configuration file, and refuses a member it cannot use in its own words.

To start on a role, choose it in the **Role** control of the new-conversation screen (or at the top of the picker). The screen says which member a conversation of that role would run on and why ("Claude · work · opus: first with usage"; the tooltip lists the members left out and why), and the first message starts that member's agent on its account and model. Picking a member by hand, in the picker of a running conversation, ends the role's hold on it.

A conversation a role serves says so under the turn: *Served by Claude · work · opus · planner*. When its member's usage runs out, the host moves the conversation to the next member by itself, and the conversation goes on in the same tab with its transcript kept. The line then says when and from whom ("planner moved from Claude to Codex at 15:04"), and hovering it lists what the move could not carry between the two agents, such as thoughts that only the agent that wrote them can read.

A role whose handover is **Ask me first** asks before a move to a different agent: "Usage ran out on Claude. Continue on Codex?" **Continue** lets the host go on; **Not now** keeps the conversation as it is and asks again when its next turn ends. A move to another account or model of the same agent never asks.
