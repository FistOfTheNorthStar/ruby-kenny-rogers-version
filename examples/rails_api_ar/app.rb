# A Rails 5.2 JSON API (ActionController::API + ActiveRecord) on SQLite, compiled by Kenny.
# The sqlite3 gem's C extension is built into Kenny's runtime; its Ruby layer comes from the gem.
require "action_controller"
require "active_record"

ActiveRecord::Base.establish_connection(adapter: "sqlite3", database: ENV["DATABASE"] || ":memory:", timeout: 5000)
ActiveRecord::Schema.verbose = false
ActiveRecord::Schema.define do
  unless table_exists?(:posts)
    create_table :posts do |t|
      t.string :title, null: false
      t.text :body
      t.integer :likes, default: 0, null: false
      t.timestamps
    end
    create_table :comments do |t|
      t.references :post, null: false
      t.text :body, null: false
      t.timestamps
    end
  end
end

class Post < ActiveRecord::Base
  has_many :comments, dependent: :destroy
  validates :title, presence: true, length: { maximum: 100 }
  scope :popular, -> { where("likes >= ?", 2).order(likes: :desc) }
end

class Comment < ActiveRecord::Base
  belongs_to :post
  validates :body, presence: true
end

class PostsController < ActionController::API
  rescue_from(ActiveRecord::RecordNotFound) { render json: { error: "not found" }, status: :not_found }

  def index
    posts = params[:popular] ? Post.popular : Post.order(:id)
    render json: posts.as_json(only: [:id, :title, :likes])
  end

  def show
    post = Post.includes(:comments).find(params[:id])
    render json: post.as_json(only: [:id, :title, :body, :likes], include: { comments: { only: [:id, :body] } })
  end

  def create
    post = Post.new(params.require(:post).permit(:title, :body))
    if post.save
      render json: post.as_json(only: [:id, :title]), status: :created
    else
      render json: { errors: post.errors.full_messages }, status: :unprocessable_entity
    end
  end

  def like
    post = Post.find(params[:id])
    post.increment!(:likes)
    render json: { id: post.id, likes: post.likes }
  end

  def destroy
    Post.find(params[:id]).destroy!
    head :no_content
  end
end

class CommentsController < ActionController::API
  def create
    comment = Post.find(params[:post_id]).comments.create!(body: params[:body])
    render json: comment.as_json(only: [:id, :post_id, :body]), status: :created
  end
end

class StatsController < ActionController::API
  def show
    render json: {
      posts: Post.count,
      comments: Comment.count,
      likes: Post.sum(:likes),
      comments_per_post: Comment.joins(:post).group("posts.title").order("posts.title").count
    }
  end
end

ROUTES = ActionDispatch::Routing::RouteSet.new
ROUTES.draw do
  resources :posts, only: [:index, :show, :create, :destroy] do
    post :like, on: :member
    resources :comments, only: [:create]
  end
  get "/stats", to: "stats#show"
end

# Hand the request thread's database connection back to the pool after each request
# (a full Rails app does this in its executor).
class ReleaseConnections
  def initialize(app)
    @app = app
  end

  def call(env)
    @app.call(env)
  ensure
    ActiveRecord::Base.clear_active_connections!
  end
end

APP = ReleaseConnections.new(ROUTES)
