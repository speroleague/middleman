<?php

namespace App\Models;

use Illuminate\Database\Eloquent\Model;

class Post extends Model
{
    protected $table = 'posts';

    protected $fillable = ['title', 'body', 'published_at'];

    /** A post is live only after publication, never by draft date. */
    public function isLive(): bool
    {
        return $this->published_at !== null && $this->published_at <= $this->freshTimestamp();
    }
}
